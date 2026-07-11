// SPDX-License-Identifier: Apache-2.0
//! License loader — reads the JWT payload from `~/.laplace/config.json` to
//! determine the `max_depth` limit.
//!
//! [GHOST CONSTRAINT]: decodes only the payload without verifying the JWT
//! signature (delegated to the server).
//! The `exp` expiry check is omitted; this is for local developer cargo tests only.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::Deserialize;
use std::path::PathBuf;

/// ~/.laplace/config.json 저장 형식.
#[derive(Deserialize)]
struct LaplaceConfig {
    jwt: String,
}

/// JWT claims에서 `axiom_max_depth`를 추출한다.
#[derive(Deserialize)]
struct ClaimsPartial {
    limits: LimitsPartial,
}

#[derive(Deserialize)]
struct LimitsPartial {
    axiom_max_depth: Option<usize>,
}

fn config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".laplace").join("config.json"))
}

/// Reads `axiom_max_depth` from `~/.laplace/config.json`.
///
/// Returns `None` when the file is missing or parsing fails; callers use a default.
#[must_use]
pub fn load_axiom_max_depth() -> Option<usize> {
    let path = config_path()?;
    let json = std::fs::read_to_string(path).ok()?;
    let config: LaplaceConfig = serde_json::from_str(&json).ok()?;

    let parts: Vec<&str> = config.jwt.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    let claims: ClaimsPartial = serde_json::from_slice(&payload_bytes).ok()?;
    claims.limits.axiom_max_depth
}
