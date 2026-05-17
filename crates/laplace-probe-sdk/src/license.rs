//! 라이선스 로더 — ~/.laplace/config.json에서 JWT payload를 읽어
//! `max_depth` 상한을 결정한다.
//!
//! [GHOST CONSTRAINT]: JWT 서명 검증 없이 payload만 디코딩한다 (서버 위임 방식).
//! exp 만료 체크는 생략한다 — 개발자 로컬 cargo test 전용.

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

/// ~/.laplace/config.json에서 `axiom_max_depth`를 읽는다.
///
/// 파일이 없거나 파싱 실패 시 `None`을 반환한다. 호출자는 기본값을 사용한다.
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
