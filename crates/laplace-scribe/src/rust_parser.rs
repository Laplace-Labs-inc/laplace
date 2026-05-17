use std::path::Path;

use regex::Regex;
use syn::visit_mut::VisitMut;

use crate::{
    config::ScribeConfig,
    context::ScribeContext,
    error::{LaplaceError, LaplaceResult},
};

/// A size-bounded knowledge chunk produced from a single Rust source file.
pub struct RsChunk {
    pub workspace: String,
    pub filename: String,
    pub content: String,
}

/// A single Rust symbol extracted from the AST, ready for DB insertion.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SymbolRecord {
    pub name: String,
    pub kind: SymbolKind,
    pub is_pub: bool,
    pub has_repr_c: bool,
    pub layer: Option<String>,
    pub link: Option<String>,
    /// Line number in the source file. Requires proc-macro2 "span-locations".
    pub line_number: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Struct,
    Enum,
    Trait,
    Fn,
    Impl,
}

impl SymbolKind {
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Fn => "fn",
            Self::Impl => "impl",
        }
    }
}

/// Parse a Rust source file with `syn`:
///   1. Apply `GhostTransformer` — all function bodies become `unimplemented!()`.
///   2. For each top-level item, prepend:
///      - `// [ABI_GUARD]: FFI Boundary` if the struct carries `#[repr(C)]`.
///      - `// [GHOST CONSTRAINT]: …`   for any constraint recorded in `ctx`.
///      - `// [LAPLACE_META] …`        if a `#[laplace::knowledge]` or
///        `#[cfg_attr(…, laplace_meta(…))]` attribute is present.
///   3. Pack rendered fragments into ≤ `cfg.chunk_limit` byte chunks.
///   4. Extract symbol metadata for DB ingestion.
pub fn parse_file(
    path: &Path,
    crate_name: &str,
    workspace: &str,
    cfg: &ScribeConfig,
    ctx: &ScribeContext,
) -> LaplaceResult<Vec<(RsChunk, Vec<SymbolRecord>)>> {
    let source = std::fs::read_to_string(path).map_err(|e| LaplaceError::Io {
        path: path.display().to_string(),
        source: e,
    })?;

    let mut syn_file: syn::File = syn::parse_str(&source).map_err(|e| LaplaceError::RustParse {
        file: path.display().to_string(),
        msg: e.to_string(),
    })?;

    // ── Ghost-body transformation ─────────────────────────────────────────────
    let mut transformer = GhostTransformer;
    transformer.visit_file_mut(&mut syn_file);

    // ── Collect symbols and render items ──────────────────────────────────
    let mut fragments: Vec<String> = Vec::new();
    let mut symbols: Vec<SymbolRecord> = Vec::new();
    for item in &syn_file.items {
        if !is_target_item(item) || is_test_item(item) {
            continue;
        }
        let frag = render_item(item, ctx);
        if !frag.trim().is_empty() {
            fragments.push(frag);
        }
        if let Some(sym) = extract_symbol_record(item) {
            symbols.push(sym);
        }
    }

    if fragments.is_empty() {
        return Ok(vec![]);
    }

    let file_stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    // src/, benches/, examples/ 는 크레이트 표준 디렉토리이므로 prefix 생략.
    // 그 외 하위 모듈 디렉토리(dpor/, adapters/, v8_ffi/ 등)는 prefix 추가하여
    // 동일 파일명(bridge.rs, mod.rs 등)의 충돌을 방지한다.
    let stem = if matches!(parent.as_str(), "src" | "benches" | "examples") {
        file_stem
    } else {
        format!("{}_{}", parent, file_stem)
    };

    let chunks = pack_chunks(fragments, workspace, crate_name, &stem, cfg.chunk_limit);

    // Attach symbols to chunk[0]; other chunks have empty symbol lists.
    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| {
            let syms = if i == 0 { symbols.clone() } else { vec![] };
            (chunk, syms)
        })
        .collect())
}

// ── Ghost Body Transformer ────────────────────────────────────────────────────

struct GhostTransformer;

impl VisitMut for GhostTransformer {
    /// Replace free-function bodies with `{ unimplemented!() }`.
    fn visit_item_fn_mut(&mut self, node: &mut syn::ItemFn) {
        node.block = Box::new(syn::parse_quote!({ unimplemented!() }));
        // Do not recurse: the new body has no nested items.
    }

    /// Replace `impl` method bodies with `{ unimplemented!() }`.
    fn visit_impl_item_fn_mut(&mut self, node: &mut syn::ImplItemFn) {
        node.block = syn::parse_quote!({ unimplemented!() });
    }

    /// Replace trait default method bodies with `{ unimplemented!() }`.
    fn visit_trait_item_fn_mut(&mut self, node: &mut syn::TraitItemFn) {
        if node.default.is_some() {
            node.default = Some(syn::parse_quote!({ unimplemented!() }));
        }
        // Methods without a default body (`fn foo(&self);`) are left as-is.
    }
}

// ── Item Filtering ────────────────────────────────────────────────────────────

/// Return `true` only for the item types we want to emit as knowledge.
/// `Item::Use`, `Item::Mod`, `Item::Macro`, `Item::Type`, etc. are excluded.
fn is_target_item(item: &syn::Item) -> bool {
    matches!(
        item,
        syn::Item::Struct(_)
            | syn::Item::Enum(_)
            | syn::Item::Trait(_)
            | syn::Item::Fn(_)
            | syn::Item::Impl(_)
    )
}

/// Return `true` if the item should be suppressed because it is test-related:
///   - carries a `#[cfg(test)]` attribute, or
///   - its name starts with `test` or `tests`.
fn is_test_item(item: &syn::Item) -> bool {
    let has_cfg_test = item_attrs(item).iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }
        match &attr.meta {
            syn::Meta::List(list) => list.tokens.to_string().trim() == "test",
            _ => false,
        }
    });

    if has_cfg_test {
        return true;
    }

    // Filter by name prefix: `test`, `tests`, `test_*`
    if let Some(name) = item_name(item) {
        return name == "test" || name == "tests" || name.starts_with("test_");
    }

    false
}

// ── Item Rendering ────────────────────────────────────────────────────────────

/// Render a single top-level `syn::Item` as an annotated, pretty-printed
/// string, prepending ABI guard and ghost constraint headers as required.
fn render_item(item: &syn::Item, ctx: &ScribeContext) -> String {
    let mut header = String::new();

    // Ghost constraint injection
    if let Some(name) = item_name(item) {
        for constraint in ctx.constraints_for(&name) {
            header.push_str(&format!("// [GHOST CONSTRAINT]: {}\n", constraint));
        }
    }

    // ABI guard for #[repr(C)] structs
    if let syn::Item::Struct(s) = item {
        if has_repr_c(&s.attrs) {
            header.push_str("// [ABI_GUARD]: FFI Boundary\n");
        }
    }

    // Laplace meta attribute → knowledge pointer comments
    if let Some(meta) = extract_laplace_meta(item_attrs(item)) {
        if let Some(layer) = meta.layer {
            header.push_str(&format!("// [LAPLACE_META] layer={}\n", layer));
        }
        if let Some(link) = meta.link {
            header.push_str(&format!("// [LINK]: {}\n", link));
        }
    }

    // Pretty-print the (transformed) item via prettyplease
    let wrapped = syn::File {
        shebang: None,
        attrs: vec![],
        items: vec![item.clone()],
    };
    let pretty = prettyplease::unparse(&wrapped);

    if header.is_empty() {
        format!("{}\n", pretty.trim())
    } else {
        format!("{}{}\n", header, pretty.trim())
    }
}

// ── Symbol Extraction ─────────────────────────────────────────────────────────

/// Extract SymbolRecord from a syn::Item.
/// Returns None if the item is not a target kind.
fn extract_symbol_record(item: &syn::Item) -> Option<SymbolRecord> {
    let name = item_name(item)?;

    let kind = match item {
        syn::Item::Struct(_) => SymbolKind::Struct,
        syn::Item::Enum(_) => SymbolKind::Enum,
        syn::Item::Trait(_) => SymbolKind::Trait,
        syn::Item::Fn(_) => SymbolKind::Fn,
        syn::Item::Impl(_) => SymbolKind::Impl,
        _ => return None,
    };

    let is_pub = match item {
        syn::Item::Struct(s) => matches!(s.vis, syn::Visibility::Public(_)),
        syn::Item::Enum(e) => matches!(e.vis, syn::Visibility::Public(_)),
        syn::Item::Trait(t) => matches!(t.vis, syn::Visibility::Public(_)),
        syn::Item::Fn(f) => matches!(f.vis, syn::Visibility::Public(_)),
        syn::Item::Impl(_) => true, // impl blocks are always "public"
        _ => false,
    };

    let has_repr_c = if let syn::Item::Struct(s) = item {
        has_repr_c(&s.attrs)
    } else {
        false
    };

    let meta = extract_laplace_meta(item_attrs(item));
    let layer = meta.as_ref().and_then(|m| m.layer.clone());
    let link = meta.as_ref().and_then(|m| m.link.clone());

    // line_number: requires proc-macro2 "span-locations" feature
    let line_number = item_span(item).map(|span| span.start().line as u32);

    Some(SymbolRecord {
        name,
        kind,
        is_pub,
        has_repr_c,
        layer,
        link,
        line_number,
    })
}

fn item_span(item: &syn::Item) -> Option<proc_macro2::Span> {
    match item {
        syn::Item::Struct(s) => Some(s.ident.span()),
        syn::Item::Enum(e) => Some(e.ident.span()),
        syn::Item::Trait(t) => Some(t.ident.span()),
        syn::Item::Fn(f) => Some(f.sig.ident.span()),
        syn::Item::Impl(i) => {
            if let syn::Type::Path(tp) = i.self_ty.as_ref() {
                tp.path.get_ident().map(|id| id.span())
            } else {
                None
            }
        }
        _ => None,
    }
}

// ── Attribute Helpers ─────────────────────────────────────────────────────────

fn item_name(item: &syn::Item) -> Option<String> {
    match item {
        syn::Item::Struct(s) => Some(s.ident.to_string()),
        syn::Item::Enum(e) => Some(e.ident.to_string()),
        syn::Item::Trait(t) => Some(t.ident.to_string()),
        syn::Item::Fn(f) => Some(f.sig.ident.to_string()),
        syn::Item::Impl(i) => {
            if let syn::Type::Path(tp) = i.self_ty.as_ref() {
                tp.path.get_ident().map(|id| id.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Struct(s) => &s.attrs,
        syn::Item::Enum(e) => &e.attrs,
        syn::Item::Trait(t) => &t.attrs,
        syn::Item::Fn(f) => &f.attrs,
        syn::Item::Impl(i) => &i.attrs,
        _ => &[],
    }
}

/// Return `true` if the attribute list contains `#[repr(C)]` (or `#[repr(C, …)]`).
fn has_repr_c(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("repr") {
            return false;
        }
        match &attr.meta {
            syn::Meta::List(list) => list
                .tokens
                .to_string()
                .split(',')
                .any(|tok| tok.trim() == "C"),
            _ => false,
        }
    })
}

// ── Laplace Meta Extraction ───────────────────────────────────────────────────

struct LaplaceMetaInfo {
    layer: Option<String>,
    link: Option<String>,
}

/// Extract `layer` and `link` fields from:
///   - `#[laplace::knowledge(layer = "…", link = "…")]`
///   - `#[cfg_attr(feature = "scribe_docs", laplace_meta(layer = "…", link = "…"))]`
fn extract_laplace_meta(attrs: &[syn::Attribute]) -> Option<LaplaceMetaInfo> {
    for attr in attrs {
        let path = attr.path();
        let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();

        if segs == ["laplace", "knowledge"] {
            if let syn::Meta::List(list) = &attr.meta {
                return Some(parse_kv_meta(&list.tokens.to_string()));
            }
        }

        if path.is_ident("cfg_attr") {
            if let syn::Meta::List(list) = &attr.meta {
                let s = list.tokens.to_string();
                if s.contains("scribe_docs") && s.contains("laplace_meta") {
                    let inner_re = Regex::new(r"laplace_meta\s*\(([^)]*)\)").unwrap();
                    if let Some(cap) = inner_re.captures(&s) {
                        return Some(parse_kv_meta(&cap[1]));
                    }
                }
            }
        }
    }
    None
}

fn parse_kv_meta(s: &str) -> LaplaceMetaInfo {
    let re = Regex::new(r#"(\w+)\s*=\s*"([^"]+)""#).unwrap();
    let mut layer = None;
    let mut link = None;
    for cap in re.captures_iter(s) {
        match &cap[1] {
            "layer" => layer = Some(cap[2].to_string()),
            "link" => link = Some(cap[2].to_string()),
            _ => {}
        }
    }
    LaplaceMetaInfo { layer, link }
}

// ── Chunk Packing ─────────────────────────────────────────────────────────────

/// Accumulate rendered item fragments into ≤ `limit`-byte Markdown chunks.
fn pack_chunks(
    fragments: Vec<String>,
    workspace: &str,
    crate_name: &str,
    stem: &str,
    limit: usize,
) -> Vec<RsChunk> {
    let mut chunks: Vec<RsChunk> = Vec::new();
    let mut current = String::new();

    for frag in fragments {
        // If adding this fragment would overflow the limit, flush first.
        if !current.is_empty() && current.len() + frag.len() > limit {
            let idx = chunks.len();
            chunks.push(make_chunk(
                workspace,
                crate_name,
                stem,
                idx,
                std::mem::take(&mut current),
            ));
        }
        current.push_str(&frag);
        current.push('\n');
    }

    if !current.trim().is_empty() {
        let idx = chunks.len();
        chunks.push(make_chunk(workspace, crate_name, stem, idx, current));
    }

    chunks
}

fn make_chunk(
    workspace: &str,
    crate_name: &str,
    stem: &str,
    index: usize,
    body: String,
) -> RsChunk {
    let frontmatter = format!(
        "---\nworkspace: {}\ncrate: {}\nchunk: {}\n---\n\n",
        workspace, crate_name, index
    );
    RsChunk {
        workspace: workspace.to_string(),
        filename: format!("{}__{}_chunk_{:02}.md", crate_name, stem, index),
        content: format!("{}```rust\n{}\n```\n", frontmatter, body.trim()),
    }
}
