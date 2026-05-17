//! `KnulRpcServer` — QUIC bi-stream 요청을 수락하는 서버 측 컴포넌트.

use crate::{read_frame, write_result, RpcError};
use std::net::SocketAddr;

pub struct KnulRpcServer {
    endpoint: quinn::Endpoint,
}

/// 단일 인바운드 RPC 호출.
pub struct KnulRpcCall {
    pub action: String,
    pub body: serde_json::Value,
    send: quinn::SendStream,
}

impl KnulRpcServer {
    /// `bind_addr`에서 QUIC 서버를 시작한다.
    ///
    /// # Errors
    ///
    /// Returns error if TLS setup or socket binding fails.
    pub fn bind(bind_addr: SocketAddr) -> Result<Self, RpcError> {
        let server_config = crate::tls::make_server_config(&["localhost", "api.laplace.rs"])?;
        let endpoint = quinn::Endpoint::server(server_config, bind_addr)
            .map_err(|e| RpcError::Bind(e.to_string()))?;
        Ok(Self { endpoint })
    }

    /// 다음 RPC 호출을 수락한다. 서버가 종료되면 `None` 반환.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] on connection, stream, or frame read failures.
    pub async fn accept(&self) -> Option<Result<KnulRpcCall, RpcError>> {
        let conn = self.endpoint.accept().await?;
        let connection = match conn.await {
            Ok(c) => c,
            Err(e) => return Some(Err(RpcError::Connect(e.to_string()))),
        };
        let (send, mut recv) = match connection.accept_bi().await {
            Ok(pair) => pair,
            Err(e) => return Some(Err(RpcError::Stream(e.to_string()))),
        };
        let (action, body) = match read_frame(&mut recv).await {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };
        Some(Ok(KnulRpcCall { action, body, send }))
    }
}

impl KnulRpcCall {
    /// 성공 응답을 전송하고 스트림을 닫는다.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] on write failure or serialization error.
    pub async fn respond_ok(mut self, body: &serde_json::Value) -> Result<(), RpcError> {
        write_result(&mut self.send, true, body).await?;
        drop(self.send); // Stream 종료
        Ok(())
    }

    /// 오류 응답을 전송하고 스트림을 닫는다.
    ///
    /// # Errors
    ///
    /// Returns [`RpcError`] on write failure or serialization error.
    pub async fn respond_err(mut self, msg: &str) -> Result<(), RpcError> {
        let err = serde_json::json!({ "error": msg });
        write_result(&mut self.send, false, &err).await?;
        drop(self.send); // Stream 종료
        Ok(())
    }
}
