// SPDX-License-Identifier: Apache-2.0
use std::path::Path;

use regex::Regex;

use crate::{
    config::ScribeConfig,
    context::ScribeContext,
    error::{LaplaceError, LaplaceResult},
};

/// A size-bounded knowledge chunk produced from a single Markdown file.
pub struct MdChunk {
    pub workspace: String,
    pub filename: String,
    pub content: String,
    /// Ghost constraint targets extracted from this chunk's source file.
    pub ghost_targets: Vec<String>,
}

/// Parse a Markdown file:
///   1. Extract YAML frontmatter and emit it in the first chunk's header.
///   2. Scan for `> [GHOST_CONSTRAINT: target=X]` blocks and populate `ctx`.
///   3. Paginate the body at `##` / `###` heading boundaries, respecting
///      the 15 KB chunk limit from `cfg`.
pub fn parse_file(
    path: &Path,
    workspace: &str,
    cfg: &ScribeConfig,
    ctx: &mut ScribeContext,
) -> LaplaceResult<Vec<MdChunk>> {
    let source = std::fs::read_to_string(path).map_err(|e| LaplaceError::Io {
        path: path.display().to_string(),
        source: e,
    })?;

    let (frontmatter, body_start) = extract_frontmatter(&source);
    extract_ghost_constraints(&source, ctx);

    let body = &source[body_start..];
    let pages = paginate(body, cfg.chunk_limit);

    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let parent_prefix = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| format!("{}_", n.to_string_lossy()))
        .unwrap_or_default();

    // ghost_targets는 파일 전체에서 추출 (chunk 0에만 기록)
    let targets: Vec<String> = ctx
        .ghost_constraints
        .keys()
        .filter(|k| source.contains(k.as_str()))
        .cloned()
        .collect();

    let chunks = pages
        .into_iter()
        .enumerate()
        .map(|(i, page)| {
            let header = if i == 0 && !frontmatter.is_empty() {
                format!("---\n{}\n---\n\n", frontmatter)
            } else {
                String::new()
            };
            MdChunk {
                workspace: workspace.to_string(),
                filename: format!("{}{}_chunk_{:02}.md", parent_prefix, stem, i),
                content: format!("{}{}", header, page),
                ghost_targets: if i == 0 { targets.clone() } else { vec![] },
            }
        })
        .collect();

    Ok(chunks)
}

// ── Frontmatter ───────────────────────────────────────────────────────────────

/// Returns `(frontmatter_yaml_content, byte_offset_of_body)`.
/// Frontmatter is the text between the first `---\n` and the next `\n---\n`.
fn extract_frontmatter(source: &str) -> (String, usize) {
    if !source.starts_with("---\n") {
        return (String::new(), 0);
    }
    if let Some(end_rel) = source[4..].find("\n---\n") {
        let fm = source[4..4 + end_rel].to_string();
        let body_start = 4 + end_rel + 5; // len("\n---\n") == 5
        return (fm, body_start);
    }
    (String::new(), 0)
}

// ── Ghost Constraint Extraction ───────────────────────────────────────────────

/// Scan `source` for blockquote lines of the form:
///   `> [GHOST_CONSTRAINT: target=StructName] optional description`
/// and record them in `ctx`.
fn extract_ghost_constraints(source: &str, ctx: &mut ScribeContext) {
    let re = Regex::new(r"(?m)^>\s*\[GHOST_CONSTRAINT:\s*target=(\w+)\]\s*(.*)")
        .expect("static regex is valid");

    for cap in re.captures_iter(source) {
        let target = &cap[1];
        let desc = cap[2].trim();
        let constraint = if desc.is_empty() {
            format!("Constraint for {}", target)
        } else {
            desc.to_string()
        };
        ctx.add_constraint(target, &constraint);
    }
}

// ── Pagination ────────────────────────────────────────────────────────────────

/// Split `text` into pages whose byte length does not exceed `limit`.
/// Splits are made at the nearest preceding `##` or `###` heading.
/// Falls back to the nearest preceding newline if no heading is found.
fn paginate(text: &str, limit: usize) -> Vec<String> {
    if text.len() <= limit {
        return vec![text.to_string()];
    }

    let heading_re = Regex::new(r"(?m)^#{2,3}\s").expect("static regex is valid");
    let heading_positions: Vec<usize> = heading_re.find_iter(text).map(|m| m.start()).collect();

    let bytes = text.as_bytes();
    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0usize;

    loop {
        if text.len() - start <= limit {
            chunks.push(text[start..].to_string());
            break;
        }

        let window_end = (start + limit).min(text.len());

        // Prefer splitting at a heading boundary within the window.
        let split_at = heading_positions
            .iter()
            .rev()
            .find(|&&pos| pos > start && pos <= window_end)
            .copied()
            .unwrap_or_else(|| {
                // Fallback: last newline within window.
                let window = &bytes[start..window_end];
                let last_nl = window.iter().rposition(|&b| b == b'\n');
                start + last_nl.unwrap_or(window.len())
            });

        if split_at <= start {
            // Safety valve: avoid infinite loop on pathological input.
            chunks.push(text[start..].to_string());
            break;
        }

        chunks.push(text[start..split_at].to_string());
        start = split_at;
    }

    chunks
}
