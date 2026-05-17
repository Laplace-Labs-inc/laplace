// SPDX-License-Identifier: Apache-2.0
//! Wire protocol encoding/decoding for semantic mesh frames.
//!
//! ## Layer summary
//!
//! | Layer | Module         | Description                                  |
//! |-------|----------------|----------------------------------------------|
//! | 1     | `dictionary`   | Static LEB128 VarInt IDs (0x0001–0x3FFF)    |
//! | 2     | `compression`  | Dynamic session dictionary (0x8000–0xFFFF)   |
//! | 3     | `compression`  | LZ4 byte-level compression (payloads > 4 KiB)|

pub mod compression;
pub mod decoder;
pub mod dictionary;
pub mod encoder;
pub mod error;
pub mod parser;

pub use compression::{
    lz4_compress, lz4_decompress, DictSyncMessage, TokenFrequencyTracker, LZ4_COMPRESSION_THRESHOLD,
};
pub use decoder::SemanticDecoder;
pub use dictionary::{read_varint, write_varint, StaticDictionary, STATIC_ID_MAX, STATIC_ID_MIN};
pub use encoder::SemanticEncoder;
pub use error::MeshError;
pub use parser::fetch_and_build_dictionary;
