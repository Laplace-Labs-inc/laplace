// SPDX-License-Identifier: Apache-2.0
use std::{
    fs,
    path::{Path, PathBuf},
};

use regex::Regex;
use walkdir::WalkDir;

use crate::{
    config::ScribeConfig,
    context::ScribeContext,
    error::{LaplaceError, LaplaceResult},
    index::{ChunkKind, ChunkRecord, IndexGenerator},
    md_parser, rust_parser,
};

#[cfg(feature = "db-ingest")]
use std::time::Instant;

/// Orchestrates the two-phase knowledge extraction pipeline:
///   Phase 1 – Markdown docs   → populates `ScribeContext` with ghost constraints.
///   Phase 2 – Rust source     → AST extraction + constraint injection.
pub struct ScribeEngine<'cfg> {
    cfg: &'cfg ScribeConfig,
    // DB 클라이언트는 --db-url 있을 때만 Some
    #[cfg(feature = "db-ingest")]
    db: Option<crate::db::LksDb>,
}

impl<'cfg> ScribeEngine<'cfg> {
    pub fn new(cfg: &'cfg ScribeConfig) -> LaplaceResult<Self> {
        #[cfg(feature = "db-ingest")]
        let db = if let Some(url) = &cfg.db_url {
            Some(crate::db::LksDb::connect(url)?)
        } else {
            None
        };

        Ok(Self {
            cfg,
            #[cfg(feature = "db-ingest")]
            db,
        })
    }

    pub fn run(&mut self) -> LaplaceResult<RunStats> {
        #[cfg(feature = "db-ingest")]
        let start_time = Instant::now();

        let mut ctx = ScribeContext::default();
        let mut stats = RunStats::default();
        let mut symbols_written = 0i64;

        #[cfg(feature = "db-ingest")]
        let mut gc_written = 0i64;
        #[cfg(feature = "db-ingest")]
        let ghost_pairs: Vec<(String, String)> = Vec::new();

        // Phase 1: Markdown docs (must run first to populate ghost constraints).
        let docs_dir = self.cfg.root.join("docs");
        if docs_dir.exists() {
            self.process_docs(&docs_dir, &mut ctx, &mut stats)?;
        } else {
            eprintln!("  [warn] docs/ not found; skipping Markdown phase");
        }

        // Phase 2: Rust crates.
        let crates_dir = self.cfg.root.join("crates");
        if crates_dir.exists() {
            self.process_crates(&crates_dir, &mut ctx, &mut stats, &mut symbols_written)?;
        } else {
            eprintln!("  [warn] crates/ not found; skipping Rust phase");
        }

        // Phase 3: Index generation or DB finalization
        #[cfg(feature = "db-ingest")]
        if let Some(db) = &mut self.db {
            gc_written = db.flush_ghost_constraints(&ctx, &ghost_pairs)?;
            let duration_ms = start_time.elapsed().as_millis() as i64;
            db.record_run(
                &stats,
                symbols_written,
                gc_written,
                duration_ms,
                "success",
                None,
            )?;
            // DB 모드에서는 00_Index.md 생성 생략
            return Ok(stats);
        }

        // 파일 모드에서만 인덱스 생성
        IndexGenerator::new(&self.cfg.output_dir).generate(&ctx)?;

        Ok(stats)
    }

    // ── Phase 1: Docs ─────────────────────────────────────────────────────────

    fn process_docs(
        &mut self,
        docs_dir: &Path,
        ctx: &mut ScribeContext,
        stats: &mut RunStats,
    ) -> LaplaceResult<()> {
        let workspace = &self.cfg.docs_workspace;

        for path in collect_files(docs_dir, "md") {
            let chunks = md_parser::parse_file(&path, workspace, self.cfg, ctx)?;

            // DB 모드
            #[cfg(feature = "db-ingest")]
            if let Some(db) = &mut self.db {
                for chunk in &chunks {
                    let source_rel_path = path
                        .strip_prefix(&self.cfg.root)
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    db.upsert_md_chunk(chunk, &source_rel_path)?;
                    stats.md_chunks += 1;
                }
                stats.md_files += 1;
                continue; // 파일 쓰기 건너뜀
            }

            // 파일 모드 (기존 동작)
            for chunk in chunks {
                self.write_chunk(&chunk.workspace, &chunk.filename, &chunk.content)?;
                ctx.record_chunk(ChunkRecord {
                    workspace: chunk.workspace.clone(),
                    filename: chunk.filename.clone(),
                    kind: ChunkKind::Markdown,
                    source: path
                        .strip_prefix(&self.cfg.root)
                        .unwrap_or(&path)
                        .display()
                        .to_string(),
                    ghost_targets: chunk.ghost_targets.clone(),
                });
                stats.md_chunks += 1;
            }
            stats.md_files += 1;
        }

        Ok(())
    }

    // ── Phase 2: Rust crates ──────────────────────────────────────────────────

    fn process_crates(
        &mut self,
        crates_dir: &Path,
        ctx: &mut ScribeContext,
        stats: &mut RunStats,
        symbols_written: &mut i64,
    ) -> LaplaceResult<()> {
        #[cfg(not(feature = "db-ingest"))]
        let _ = symbols_written;

        let crate_roots = discover_crates(crates_dir);

        for crate_root in crate_roots {
            let crate_name = read_crate_name(&crate_root);
            let workspace = self.cfg.resolve_workspace(&crate_name).to_string();

            for path in collect_files_filtered(&crate_root, "rs") {
                match rust_parser::parse_file(&path, &crate_name, &workspace, self.cfg, ctx) {
                    Ok(chunk_pairs) => {
                        // DB 모드
                        #[cfg(feature = "db-ingest")]
                        if let Some(db) = &mut self.db {
                            for (chunk, syms) in &chunk_pairs {
                                let source_rel_path = path
                                    .strip_prefix(&self.cfg.root)
                                    .unwrap_or(&path)
                                    .display()
                                    .to_string();
                                db.upsert_rs_chunk(chunk, syms, &source_rel_path, &crate_name)?;
                                *symbols_written += syms.len() as i64;
                                stats.rs_chunks += 1;
                            }
                            stats.rs_files += 1;
                            continue;
                        }

                        // 파일 모드 (기존 동작)
                        for (chunk, _symbols) in chunk_pairs {
                            self.write_chunk(&chunk.workspace, &chunk.filename, &chunk.content)?;
                            ctx.record_chunk(ChunkRecord {
                                workspace: chunk.workspace.clone(),
                                filename: chunk.filename.clone(),
                                kind: ChunkKind::Rust,
                                source: path
                                    .strip_prefix(&self.cfg.root)
                                    .unwrap_or(&path)
                                    .display()
                                    .to_string(),
                                ghost_targets: vec![],
                            });
                            stats.rs_chunks += 1;
                        }
                        stats.rs_files += 1;
                    }
                    Err(LaplaceError::RustParse { file, msg }) => {
                        eprintln!("  [warn] skipping {}: {}", file, msg);
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        Ok(())
    }

    // ── Writer ────────────────────────────────────────────────────────────────

    fn write_chunk(&self, workspace: &str, filename: &str, content: &str) -> LaplaceResult<()> {
        let dir = self.cfg.output_dir.join(workspace);
        fs::create_dir_all(&dir).map_err(|e| LaplaceError::DirCreate {
            path: dir.display().to_string(),
            source: e,
        })?;

        let out = dir.join(filename);
        fs::write(&out, content).map_err(|e| LaplaceError::Io {
            path: out.display().to_string(),
            source: e,
        })?;

        Ok(())
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Find all crate roots (directories containing a `Cargo.toml`) under `dir`,
/// up to 3 levels deep.
fn discover_crates(dir: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = WalkDir::new(dir)
        .min_depth(1)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && e.file_name().to_string_lossy() == "Cargo.toml")
        .filter_map(|e| e.path().parent().map(Path::to_path_buf))
        .collect();

    roots.sort();
    roots
}

/// Read the `name` field from a crate's `Cargo.toml`; falls back to the
/// directory name when the field cannot be parsed.
fn read_crate_name(crate_dir: &Path) -> String {
    let cargo_toml = crate_dir.join("Cargo.toml");
    if let Ok(content) = fs::read_to_string(&cargo_toml) {
        let re = Regex::new(r#"(?m)^\s*name\s*=\s*"([^"]+)""#).unwrap();
        if let Some(cap) = re.captures(&content) {
            return cap[1].to_string();
        }
    }
    crate_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// Collect all files with `ext` under `dir`, sorted by path.
fn collect_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().map(|x| x == ext).unwrap_or(false))
        .map(|e| e.into_path())
        .collect();
    files.sort();
    files
}

/// Like `collect_files` but skips `target/` and `tests/` directories.
fn collect_files_filtered(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().map(|x| x == ext).unwrap_or(false))
        .filter(|e| {
            !e.path().components().any(|c| {
                let s = c.as_os_str().to_string_lossy();
                s == "target" || s == "tests"
            })
        })
        .map(|e| e.into_path())
        .collect();
    files.sort();
    files
}

// ── Stats ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct RunStats {
    pub md_files: usize,
    pub md_chunks: usize,
    pub rs_files: usize,
    pub rs_chunks: usize,
}
