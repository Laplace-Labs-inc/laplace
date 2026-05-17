//! `KnulRpcClient` — QUIC 양방향 스트림을 이용한 RPC 클라이언트.
//!
//! 각 `call()`은 새로운 QUIC 연결의 bi-stream 1개를 사용한다.
//! 연결 풀링은 Phase 7에서 추가 예정.

use crate::{read_response, write_frame, RpcError};
use std::net::SocketAddr;

pub struct KnulRpcClient {
    server_addr: SocketAddr,
    server_name: String,
}

impl KnulRpcClient {
    /// 서버 주소와 TLS SNI 이름으로 클라이언트를 생성한다.
    ///
    /// # Panics
    ///
    /// Never panics under normal conditions.
    pub fn new(server_addr: SocketAddr, server_name: impl Into<String>) -> Self {
        Self {
            server_addr,
            server_name: server_name.into(),
        }
    }

    /// action + body JSON을 서버로 전송하고 응답 JSON을 반환한다.
    ///
    /// # Errors
    ///
    /// Returns `RpcError` on connection, TLS, protocol, or stream failures.
    pub async fn call(
        &self,
        action: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let endpoint = self.make_endpoint()?;
        let connection = endpoint
            .connect(self.server_addr, &self.server_name)
            .map_err(|e| RpcError::Connect(e.to_string()))?
            .await
            .map_err(|e| RpcError::Connect(e.to_string()))?;

        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| RpcError::Stream(e.to_string()))?;

        write_frame(&mut send, action, body).await?;
        drop(send); // Stream 종료

        let result = read_response(&mut recv).await?;
        Ok(result)
    }

    #[allow(clippy::unused_self)]
    fn make_endpoint(&self) -> Result<quinn::Endpoint, RpcError> {
        let client_config = crate::tls::make_client_config_skip_verify()?;
        let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse().unwrap())
            .map_err(|e| RpcError::Bind(e.to_string()))?;
        endpoint.set_default_client_config(client_config);
        Ok(endpoint)
    }
}
