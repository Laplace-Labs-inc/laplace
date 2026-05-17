use std::{collections::HashMap, fs, path::Path};

use crate::{
    context::ScribeContext,
    error::{LaplaceError, LaplaceResult},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkKind {
    Markdown,
    Rust,
}

/// Metadata recorded for every chunk written by the engine.
#[derive(Debug, Clone)]
pub struct ChunkRecord {
    pub workspace: String,
    pub filename: String,
    pub kind: ChunkKind,
    /// Original source path relative to project root.
    pub source: String,
    /// Struct/trait names this chunk constrains (Markdown only).
    pub ghost_targets: Vec<String>,
}

/// Writes per-workspace and root 00_Index.md files.
pub struct IndexGenerator<'a> {
    output_dir: &'a Path,
}

impl<'a> IndexGenerator<'a> {
    pub fn new(output_dir: &'a Path) -> Self {
        Self { output_dir }
    }

    pub fn generate(&self, ctx: &ScribeContext) -> LaplaceResult<()> {
        let records = &ctx.chunk_records;
        if records.is_empty() {
            return Ok(());
        }

        // Group by workspace.
        let mut by_workspace: HashMap<&str, Vec<&ChunkRecord>> = HashMap::new();
        for rec in records {
            by_workspace
                .entry(rec.workspace.as_str())
                .or_default()
                .push(rec);
        }

        // Per-workspace index.
        for (workspace, chunks) in &by_workspace {
            self.write_workspace_index(workspace, chunks)?;
        }

        // Root index.
        self.write_root_index(&by_workspace, ctx)?;

        Ok(())
    }

    // ── Per-workspace 00_Index.md ─────────────────────────────────────────────

    fn write_workspace_index(&self, workspace: &str, chunks: &[&ChunkRecord]) -> LaplaceResult<()> {
        let dir = self.output_dir.join(workspace);
        fs::create_dir_all(&dir).map_err(|e| LaplaceError::DirCreate {
            path: dir.display().to_string(),
            source: e,
        })?;

        let mut md = format!("# {workspace} — Chunk Index\n\n");
        md.push_str(&format!("Total: {} chunks\n\n", chunks.len()));

        // Rust chunks table.
        let rust_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Rust)
            .collect();
        if !rust_chunks.is_empty() {
            md.push_str("## Rust Chunks\n\n");
            md.push_str("| Chunk | Source |\n|-------|--------|\n");
            for c in &rust_chunks {
                md.push_str(&format!("| {} | `{}` |\n", c.filename, c.source));
            }
            md.push('\n');
        }

        // Markdown chunks table.
        let doc_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Markdown)
            .collect();
        if !doc_chunks.is_empty() {
            md.push_str("## Documentation Chunks\n\n");
            md.push_str("| Chunk | Source | Ghost Targets |\n|-------|--------|---------------|\n");
            for c in &doc_chunks {
                let targets = if c.ghost_targets.is_empty() {
                    "—".to_string()
                } else {
                    c.ghost_targets.join(", ")
                };
                md.push_str(&format!(
                    "| {} | `{}` | {} |\n",
                    c.filename, c.source, targets
                ));
            }
            md.push('\n');
        }

        let out = dir.join("00_Index.md");
        fs::write(&out, md).map_err(|e| LaplaceError::Io {
            path: out.display().to_string(),
            source: e,
        })?;

        Ok(())
    }

    // ── Root 00_Index.md ──────────────────────────────────────────────────────

    fn write_root_index(
        &self,
        by_workspace: &HashMap<&str, Vec<&ChunkRecord>>,
        ctx: &ScribeContext,
    ) -> LaplaceResult<()> {
        let mut md = String::from("# LKS Navigation Index\n\n");

        // Workspace summary table.
        md.push_str("## Workspaces\n\n");
        md.push_str("| Workspace | Rust | Docs | Total |\n");
        md.push_str("|-----------|------|------|-------|\n");

        let mut workspaces: Vec<&str> = by_workspace.keys().copied().collect();
        workspaces.sort();
        for ws in &workspaces {
            let chunks = &by_workspace[ws];
            let rust = chunks.iter().filter(|c| c.kind == ChunkKind::Rust).count();
            let docs = chunks
                .iter()
                .filter(|c| c.kind == ChunkKind::Markdown)
                .count();
            md.push_str(&format!(
                "| [{ws}](./{ws}/00_Index.md) | {rust} | {docs} | {} |\n",
                rust + docs
            ));
        }
        md.push('\n');

        // Ghost constraint cross-links.
        let cross_links = build_cross_links(ctx);
        if !cross_links.is_empty() {
            md.push_str("## Ghost Constraint Cross-Links\n\n");
            md.push_str("> Doc → Code links derived from `GHOST_CONSTRAINT` directives.\n\n");
            md.push_str("| Target (Code) | Constrained By (Doc) |\n|---------------|---------------------|\n");
            let mut targets: Vec<_> = cross_links.iter().collect();
            targets.sort_by_key(|(k, _)| *k);
            for (target, sources) in targets {
                md.push_str(&format!("| `{}` | {} |\n", target, sources.join(", ")));
            }
            md.push('\n');
        }

        let out = self.output_dir.join("00_Index.md");
        fs::write(&out, md).map_err(|e| LaplaceError::Io {
            path: out.display().to_string(),
            source: e,
        })?;

        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build target → [source_chunk_filename] map from chunk records.
fn build_cross_links(ctx: &ScribeContext) -> HashMap<&str, Vec<String>> {
    let mut links: HashMap<&str, Vec<String>> = HashMap::new();
    for rec in &ctx.chunk_records {
        for target in &rec.ghost_targets {
            links
                .entry(target.as_str())
                .or_default()
                .push(format!("`{}`", rec.filename));
        }
    }
    links
}
