#![deny(clippy::all, clippy::pedantic)]

//! `laplace-rpc` — Laplace QUIC RPC transport 추상화 및 QUIC bi-stream 기반 RPC.
//!
//! quinn API를 이 크레이트에만 격리한다.
//! quinn 버전 변경 시 이 파일만 수정하면 된다.

pub mod client;
pub mod server;
pub mod tls;

pub use client::KnulRpcClient;
pub use server::{KnulRpcCall, KnulRpcServer};

#[allow(unused_imports)]
use tokio::io::{AsyncReadExt, AsyncWriteExt}; // AsyncReadExt/AsyncWriteExt for read_exact/write_all

// ── 오류 타입 ─────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("TLS error: {0}")]
    Tls(String),
    #[error("Bind error: {0}")]
    Bind(String),
    #[error("Connect error: {0}")]
    Connect(String),
    #[error("Stream error: {0}")]
    Stream(String),
    #[error("Protocol error: {0}")]
    Protocol(String),
    #[error("Server error: {0}")]
    ServerError(String),
}

// ── Wire Protocol 헬퍼 (내부 전용) ───────────────────────────────────────────

/// [4 BE `u32`: `action_len`][action][4 BE `u32`: `body_len`][body] 형식으로 전송
///
/// # Errors
///
/// Returns error on stream write failure or serialization error.
#[allow(clippy::cast_possible_truncation)]
pub(crate) async fn write_frame(
    send: &mut quinn::SendStream,
    action: &str,
    body: &serde_json::Value,
) -> Result<(), RpcError> {
    let body_bytes = serde_json::to_vec(body).map_err(|e| RpcError::Protocol(e.to_string()))?;
    let action_bytes = action.as_bytes();

    send.write_all(&(action_bytes.len() as u32).to_be_bytes())
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;
    send.write_all(action_bytes)
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;
    send.write_all(&(body_bytes.len() as u32).to_be_bytes())
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;
    send.write_all(&body_bytes)
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;
    Ok(())
}

/// [1 byte: ok/err][4 BE u32: body_len][body] 형식 수신
///
/// # Errors
///
/// Returns [`RpcError`] on read failure, protocol violation, or payload > 4MB.
pub(crate) async fn read_response(
    recv: &mut quinn::RecvStream,
) -> Result<serde_json::Value, RpcError> {
    let mut status = [0u8; 1];
    recv.read_exact(&mut status)
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;

    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;
    let body_len = u32::from_be_bytes(len_buf) as usize;
    if body_len > 4 * 1024 * 1024 {
        return Err(RpcError::Protocol("response too large".into()));
    }

    let mut body = vec![0u8; body_len];
    recv.read_exact(&mut body)
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;

    if status[0] != 0x00 {
        let msg = String::from_utf8_lossy(&body);
        return Err(RpcError::ServerError(msg.into_owned()));
    }

    serde_json::from_slice(&body).map_err(|e| RpcError::Protocol(e.to_string()))
}

/// [action][body] 형식 수신
///
/// # Errors
///
/// Returns [`RpcError`] on read failure, action length > 256, or payload > 4MB.
pub(crate) async fn read_frame(
    recv: &mut quinn::RecvStream,
) -> Result<(String, serde_json::Value), RpcError> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;
    let action_len = u32::from_be_bytes(len_buf) as usize;
    if action_len > 256 {
        return Err(RpcError::Protocol("action too long".into()));
    }
    let mut action_buf = vec![0u8; action_len];
    recv.read_exact(&mut action_buf)
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;
    let action = String::from_utf8(action_buf).map_err(|e| RpcError::Protocol(e.to_string()))?;

    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;
    let body_len = u32::from_be_bytes(len_buf) as usize;
    if body_len > 4 * 1024 * 1024 {
        return Err(RpcError::Protocol("body too large".into()));
    }
    let mut body_buf = vec![0u8; body_len];
    recv.read_exact(&mut body_buf)
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;
    let body = serde_json::from_slice(&body_buf).map_err(|e| RpcError::Protocol(e.to_string()))?;

    Ok((action, body))
}

/// 응답 전송
#[allow(clippy::cast_possible_truncation)]
pub(crate) async fn write_result(
    send: &mut quinn::SendStream,
    ok: bool,
    body: &serde_json::Value,
) -> Result<(), RpcError> {
    let body_bytes = serde_json::to_vec(body).map_err(|e| RpcError::Protocol(e.to_string()))?;
    send.write_all(&[u8::from(!ok)])
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;
    send.write_all(&(body_bytes.len() as u32).to_be_bytes())
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;
    send.write_all(&body_bytes)
        .await
        .map_err(|e| RpcError::Stream(e.to_string()))?;
    Ok(())
}
