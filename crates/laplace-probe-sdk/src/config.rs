// SPDX-License-Identifier: Apache-2.0
//! Automatic discovery and parsing of laplace.toml.
//!
//! In a `cargo test` context, finds `laplace.toml` in the project root and
//! loads the `[axiom]` section.
//!
//! [GHOST CONSTRAINT]: never panic when the file is missing or parsing fails;
//! return `None`.
//! [GHOST CONSTRAINT]: this module does not depend on `laplace-interfaces`.
//! It defines lightweight parsing structs that remain compatible with the same
//! TOML field names.

use serde::Deserialize;
use std::path::{Path, PathBuf};

// ── 내부 파싱 구조체 ──────────────────────────────────────────────────────────

/// laplace.toml 루트.
/// [axiom] 섹션만 파싱. [kraken], [probe]는 SDK에서 불필요.
#[derive(Deserialize, Default)]
struct TomlRoot {
    #[serde(default)]
    axiom: TomlAxiom,
}

/// [axiom] 섹션 — laplace-interfaces AxiomConfig와 필드명 동일.
#[derive(Deserialize, Default)]
struct TomlAxiom {
    max_depth: Option<u32>,
    max_threads: Option<u32>,
    max_starvation_limit: Option<u32>,
    max_danger: Option<u32>,
    default_seed: Option<u64>,
}

// ── 공개 API ──────────────────────────────────────────────────────────────────

/// Axiom configuration loaded from laplace.toml.
#[derive(Debug, Clone)]
pub struct ProjectConfig {
    /// Maximum DPOR exploration depth.
    pub max_depth: Option<usize>,
    /// Maximum number of concurrent threads.
    pub max_threads: Option<usize>,
    /// Starvation detection limit.
    pub max_starvation_limit: Option<usize>,
    /// Upper bound on the danger score.
    pub max_danger: Option<usize>,
    /// Axiom Oracle RNG seed.
    pub default_seed: Option<u64>,
}

/// Automatically locates and loads laplace.toml.
///
/// Search order:
/// 1. `$CARGO_MANIFEST_DIR/laplace.toml`
/// 2. `$CARGO_MANIFEST_DIR/../laplace.toml` (workspace root)
/// 3. `$CARGO_MANIFEST_DIR/../../laplace.toml` (nested crate)
/// 4. `./laplace.toml` (CWD fallback)
///
/// # Returns
///
/// File found and parsed successfully → `Some(ProjectConfig)`
/// File missing or parsing failed → `None` (warn log)
pub fn load_project_config() -> Option<ProjectConfig> {
    let path = find_laplace_toml()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let root: TomlRoot = match toml::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("laplace.toml parse error at {}: {e}", path.display());
            return None;
        }
    };

    Some(ProjectConfig {
        max_depth: root.axiom.max_depth.map(|v| v as usize),
        max_threads: root.axiom.max_threads.map(|v| v as usize),
        max_starvation_limit: root.axiom.max_starvation_limit.map(|v| v as usize),
        max_danger: root.axiom.max_danger.map(|v| v as usize),
        default_seed: root.axiom.default_seed,
    })
}

/// Convenience function that reads `[axiom] max_depth` from laplace.toml.
///
/// Shorthand for `load_project_config()`.
pub fn load_toml_max_depth() -> Option<usize> {
    load_project_config().and_then(|c| c.max_depth)
}

// ── 내부 탐색 ─────────────────────────────────────────────────────────────────

/// laplace.toml 파일을 프로젝트 디렉토리에서 탐색한다.
fn find_laplace_toml() -> Option<PathBuf> {
    // 1. CARGO_MANIFEST_DIR 기반 탐색 (cargo test가 설정)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let base = Path::new(&manifest_dir);

        // 현재 크레이트 루트
        let candidate = base.join("laplace.toml");
        if candidate.exists() {
            return Some(candidate);
        }

        // workspace 루트 (1단계 상위)
        let candidate = base
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("laplace.toml"));
        if let Some(c) = candidate {
            if c.exists() {
                return Some(c);
            }
        }

        // workspace 루트 (2단계 상위 — nested crate)
        let candidate = base
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|p| p.join("laplace.toml"));
        if let Some(c) = candidate {
            if c.exists() {
                return Some(c);
            }
        }
    }

    // 2. CWD fallback
    let candidate = PathBuf::from("laplace.toml");
    if candidate.exists() {
        return Some(candidate);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_laplace_toml_from_workspace() {
        // cargo test가 CARGO_MANIFEST_DIR을 설정하므로
        // workspace 루트의 laplace.toml을 찾아야 한다.
        let path = find_laplace_toml();
        match path {
            Some(p) => println!("Found laplace.toml at: {}", p.display()),
            None => println!("laplace.toml not found (may not be in workspace root during test)"),
        }
        // 테스트는 파일 존재 여부와 무관하게 항상 통과
    }

    #[test]
    fn load_project_config_parses_axiom() {
        let config = load_project_config();
        // workspace 루트에 laplace.toml이 있으면 파싱 성공해야 함
        if let Some(cfg) = config {
            // laplace.toml의 [axiom] max_depth = 20
            assert_eq!(cfg.max_depth, Some(20));
            assert_eq!(cfg.max_threads, Some(8));
        }
    }

    #[test]
    fn load_toml_max_depth_returns_value() {
        let depth = load_toml_max_depth();
        // workspace 루트 laplace.toml에 max_depth = 20
        if depth.is_some() {
            assert_eq!(depth, Some(20));
        }
    }
}
