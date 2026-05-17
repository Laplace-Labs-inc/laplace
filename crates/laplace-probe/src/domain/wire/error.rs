// SPDX-License-Identifier: Apache-2.0
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MeshError {
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Invalid schema: {0}")]
    InvalidSchema(String),

    #[error("Capacity exceeded: static dictionary is full (max 0x3FFF entries)")]
    CapacityExceeded,

    #[error("Endpoint not found in dictionary: {0}")]
    EndpointNotFound(String),

    #[error("Invalid frame: must be at least 1 byte")]
    InvalidFrame,

    #[error("Unknown ID in dictionary: 0x{0:04X}")]
    UnknownId(u32),

    #[error("KNUL transport error: {0}")]
    KnulError(String),

    #[error("Compression error: {0}")]
    CompressionError(String),

    #[error("Decompression error: {0}")]
    DecompressionError(String),
}
