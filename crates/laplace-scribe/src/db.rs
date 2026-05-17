// SPDX-License-Identifier: Apache-2.0
// src/db.rs
// Compiled only when `db-ingest` feature is active.

use postgres::{Client, NoTls};

use crate::{
    context::ScribeContext,
    engine::RunStats,
    error::{LaplaceError, LaplaceResult},
    md_parser::MdChunk,
    rust_parser::{RsChunk, SymbolRecord},
};

pub struct LksDb {
    client: Client,
}

impl LksDb {
    /// Open a synchronous connection to the LKS PostgreSQL instance.
    pub fn connect(db_url: &str) -> LaplaceResult<Self> {
        let client =
            Client::connect(db_url, NoTls).map_err(|e| LaplaceError::Db { msg: e.to_string() })?;
        Ok(Self { client })
    }

    /// Insert or update a Rust chunk + its symbols.
    ///
    /// Uses INSERT ... ON CONFLICT (chunk_path) DO UPDATE for idempotency.
    pub fn upsert_rs_chunk(
        &mut self,
        chunk: &RsChunk,
        symbols: &[SymbolRecord],
        source_path: &str,
        crate_name: &str,
    ) -> LaplaceResult<()> {
        // lks_chunks upsert
        self.client
            .execute(
                "INSERT INTO lks_chunks
                (workspace, crate_name, chunk_path, source_path, kind,
                 content, byte_size, chunk_index, ghost_constraints, abi_guards)
             VALUES ($1, $2, $3, $4, 'rust', $5, $6, $7, $8, $9)
             ON CONFLICT (chunk_path) DO UPDATE SET
                content    = EXCLUDED.content,
                byte_size  = EXCLUDED.byte_size,
                updated_at = NOW()",
                &[
                    &chunk.workspace,
                    &crate_name,
                    &chunk.filename,
                    &source_path,
                    &chunk.content,
                    &(chunk.content.len() as i32),
                    &(chunk_index_from_filename(&chunk.filename) as i32),
                    &(vec![] as Vec<String>), // Rust 청크는 ghost_constraints 없음
                    &abi_guards_from_content(&chunk.content),
                ],
            )
            .map_err(|e| LaplaceError::Db { msg: e.to_string() })?;

        // lks_symbols upsert (chunk[0]에만 symbol이 있음)
        for sym in symbols {
            self.client
                .execute(
                    "INSERT INTO lks_symbols
                    (name, workspace, crate_name, chunk_path, source_path,
                     line_number, kind, is_pub, has_repr_c, layer, link)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT (name, workspace) DO UPDATE SET
                    chunk_path  = EXCLUDED.chunk_path,
                    source_path = EXCLUDED.source_path,
                    line_number = EXCLUDED.line_number,
                    kind        = EXCLUDED.kind,
                    is_pub      = EXCLUDED.is_pub,
                    has_repr_c  = EXCLUDED.has_repr_c,
                    layer       = EXCLUDED.layer,
                    link        = EXCLUDED.link",
                    &[
                        &sym.name,
                        &chunk.workspace,
                        &crate_name,
                        &chunk.filename,
                        &source_path,
                        &sym.line_number.map(|n| n as i32),
                        &sym.kind.as_str(),
                        &sym.is_pub,
                        &sym.has_repr_c,
                        &sym.layer,
                        &sym.link,
                    ],
                )
                .map_err(|e| LaplaceError::Db { msg: e.to_string() })?;
        }

        Ok(())
    }

    /// Insert or update a Markdown chunk + its ghost constraints.
    pub fn upsert_md_chunk(&mut self, chunk: &MdChunk, source_path: &str) -> LaplaceResult<()> {
        // lks_chunks upsert
        self.client
            .execute(
                "INSERT INTO lks_chunks
                (workspace, crate_name, chunk_path, source_path, kind,
                 content, byte_size, chunk_index, ghost_constraints, abi_guards)
             VALUES ($1, '', $2, $3, 'markdown', $4, $5, $6, $7, '{}')
             ON CONFLICT (chunk_path) DO UPDATE SET
                content    = EXCLUDED.content,
                byte_size  = EXCLUDED.byte_size,
                updated_at = NOW()",
                &[
                    &chunk.workspace,
                    &chunk.filename,
                    &source_path,
                    &chunk.content,
                    &(chunk.content.len() as i32),
                    &(chunk_index_from_filename(&chunk.filename) as i32),
                    &chunk.ghost_targets,
                ],
            )
            .map_err(|e| LaplaceError::Db { msg: e.to_string() })?;

        // lks_ghost_constraints (chunk[0]에만 ghost_targets 있음)
        for target in &chunk.ghost_targets {
            // ScribeContext에 있는 constraint text를 찾아야 하므로
            // 여기서는 target 이름만 기록하고, constraint_text는 빈 문자열로 placeholder
            // 실제 constraint text는 engine.rs에서 ctx를 참조해서 전달한다.
            // (아래 upsert_ghost_constraint 별도 메서드 참조)
            let _ = target; // engine.rs의 flush_ghost_constraints()에서 처리
        }

        Ok(())
    }

    /// Flush all ghost constraints from ScribeContext after both phases complete.
    pub fn flush_ghost_constraints(
        &mut self,
        ctx: &ScribeContext,
        chunk_records: &[(String, String)], // (target_name, doc_chunk_path)
    ) -> LaplaceResult<i64> {
        let mut written = 0i64;
        for (target, doc_chunk_path) in chunk_records {
            for constraint_text in ctx.constraints_for(target) {
                self.client
                    .execute(
                        "INSERT INTO lks_ghost_constraints
                        (target_name, doc_chunk_path, constraint_text, workspace)
                     VALUES ($1, $2, $3, 'Laplace-Labs-Docs')
                     ON CONFLICT DO NOTHING",
                        &[target, doc_chunk_path, &constraint_text],
                    )
                    .map_err(|e| LaplaceError::Db { msg: e.to_string() })?;
                written += 1;
            }
        }
        Ok(written)
    }

    /// Record the ingest run statistics.
    pub fn record_run(
        &mut self,
        stats: &RunStats,
        symbols_written: i64,
        gc_written: i64,
        duration_ms: i64,
        status: &str,
        error_msg: Option<&str>,
    ) -> LaplaceResult<()> {
        self.client
            .execute(
                "INSERT INTO lks_ingest_runs
                (md_files, md_chunks, rs_files, rs_chunks,
                 symbols_written, ghost_constraints_written, edges_written,
                 duration_ms, status, error_message)
             VALUES ($1, $2, $3, $4, $5, $6, 0, $7, $8, $9)",
                &[
                    &(stats.md_files as i32),
                    &(stats.md_chunks as i32),
                    &(stats.rs_files as i32),
                    &(stats.rs_chunks as i32),
                    &(symbols_written as i32),
                    &(gc_written as i32),
                    &(duration_ms as i32),
                    &status,
                    &error_msg,
                ],
            )
            .map_err(|e| LaplaceError::Db { msg: e.to_string() })?;
        Ok(())
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Extract chunk index from filename: `..._chunk_03.md` → 3
fn chunk_index_from_filename(filename: &str) -> usize {
    let re = regex::Regex::new(r"_chunk_(\d+)\.md$").unwrap();
    re.captures(filename)
        .and_then(|c| c[1].parse().ok())
        .unwrap_or(0)
}

/// Extract ABI guard struct names from chunk content.
/// Looks for lines: `// [ABI_GUARD]: FFI Boundary` followed by `pub struct Name`
fn abi_guards_from_content(content: &str) -> Vec<String> {
    let re = regex::Regex::new(r"// \[ABI_GUARD\]: FFI Boundary\n(?:// [^\n]*\n)*pub struct (\w+)")
        .unwrap();
    re.captures_iter(content)
        .map(|c| c[1].to_string())
        .collect()
}
