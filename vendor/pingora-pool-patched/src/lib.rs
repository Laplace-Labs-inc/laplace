// Copyright 2026 Cloudflare, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// Laplace 수정 고지: 0.8.1 원문의 구조를 보존하고, connection/lru 모듈의
// Tokio async 동기화·시간 primitive만 Laplace 결정론 hook으로 alias한다.
// 원문 `#![warn(clippy::all)]`은 vendored 코드 관례(bb8-patched, crossbeam-patched)에
// 따라 allow로 교체 — upstream 원문이 clippy 버전 상승마다 -D warnings에 걸리는 것 방지.

//! Generic connection pooling
//!
//! The pool is optimized for high concurrency, high RPS use cases. Each connection group has a
//! lock free hot pool to reduce the lock contention when some connections are reused and released
//! very frequently.

#![allow(clippy::all, clippy::pedantic)]
#![allow(clippy::new_without_default)]
#![allow(clippy::type_complexity)]
#![allow(clippy::uninlined_format_args)]

mod connection;
mod lru;

pub use connection::{ConnectionMeta, ConnectionPool, PoolNode};
