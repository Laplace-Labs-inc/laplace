// SPDX-License-Identifier: Apache-2.0
use std::path::PathBuf;

/// Maximum byte size per knowledge chunk (15 KB).
pub const CHUNK_LIMIT_BYTES: usize = 15 * 1024;

/// Maps a crate name pattern (substring match) to an LKS workspace folder.
#[derive(Debug, Clone)]
pub struct RouteRule {
    /// Substring matched against the crate package name.
    pub pattern: String,
    /// Target LKS workspace directory name under `output_dir`.
    pub workspace: String,
}

/// Top-level configuration for the scribe engine.
#[derive(Debug, Clone)]
pub struct ScribeConfig {
    /// Absolute path to the project root.
    pub root: PathBuf,
    /// Directory where LKS chunk files are written (default: `<root>/LKS`).
    pub output_dir: PathBuf,
    /// Crate-to-workspace routing rules; evaluated in order, first match wins.
    pub routes: Vec<RouteRule>,
    /// Byte threshold at which a chunk is finalised and a new one is started.
    pub chunk_limit: usize,
    /// LKS workspace used for documentation files.
    pub docs_workspace: String,
    /// PostgreSQL connection URL. When Some, engine writes to DB instead of files.
    #[allow(dead_code)]
    pub db_url: Option<String>,
}

impl ScribeConfig {
    pub fn new(root: PathBuf) -> Self {
        let output_dir = root.join("LKS");
        Self {
            output_dir,
            routes: default_routes(),
            chunk_limit: CHUNK_LIMIT_BYTES,
            docs_workspace: "Laplace-Labs-Docs".to_string(),
            root,
            db_url: None,
        }
    }

    /// Return the workspace name for a given crate, using first-match routing.
    pub fn resolve_workspace(&self, crate_name: &str) -> &str {
        for rule in &self.routes {
            if crate_name.contains(rule.pattern.as_str()) {
                return &rule.workspace;
            }
        }
        "Laplace-Labs-Misc"
    }
}

fn default_routes() -> Vec<RouteRule> {
    vec![
        RouteRule {
            pattern: "laplace-core".into(),
            workspace: "Laplace-Labs-SSOT".into(),
        },
        RouteRule {
            pattern: "laplace-interfaces".into(),
            workspace: "Laplace-Labs-SSOT".into(),
        },
        RouteRule {
            pattern: "laplace-macro".into(),
            workspace: "Laplace-Labs-SSOT".into(),
        },
        RouteRule {
            pattern: "laplace-axiom".into(),
            workspace: "Laplace-Labs-Axiom".into(),
        },
        RouteRule {
            pattern: "laplace-kernel".into(),
            workspace: "Laplace-Labs-Axiom".into(),
        },
        RouteRule {
            pattern: "laplace-harness".into(),
            workspace: "Laplace-Labs-Axiom".into(),
        },
        RouteRule {
            pattern: "laplace-probe".into(),
            workspace: "Laplace-Labs-Probe".into(),
        },
        RouteRule {
            pattern: "laplace-mesh".into(),
            workspace: "Laplace-Labs-Probe".into(),
        },
        RouteRule {
            pattern: "laplace-kraken".into(),
            workspace: "Laplace-Labs-Kraken".into(),
        },
        RouteRule {
            pattern: "laplace-console".into(),
            workspace: "Laplace-Labs-Console".into(),
        },
        RouteRule {
            pattern: "laplace-cli".into(),
            workspace: "Laplace-Labs-Console".into(),
        },
    ]
}
