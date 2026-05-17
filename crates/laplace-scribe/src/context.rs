// SPDX-License-Identifier: Apache-2.0
use crate::index::ChunkRecord;
use std::collections::HashMap;

/// Shared state threaded through both the Markdown and Rust parsers.
///
/// The Markdown parser populates `ghost_constraints` by scanning for
/// `> [GHOST_CONSTRAINT: target=StructName]` blockquote directives.
/// The Rust parser then reads this map and injects the constraint comments
/// above matching struct/enum/trait items.
#[derive(Debug, Default)]
pub struct ScribeContext {
    pub ghost_constraints: HashMap<String, Vec<String>>,
    /// All chunk records accumulated during extraction (used for index generation).
    pub chunk_records: Vec<ChunkRecord>,
}

impl ScribeContext {
    /// Record a constraint string for the named target type.
    pub fn add_constraint(&mut self, target: &str, constraint: &str) {
        self.ghost_constraints
            .entry(target.to_string())
            .or_default()
            .push(constraint.to_string());
    }

    /// Return all recorded constraints for `name`, or an empty slice.
    pub fn constraints_for(&self, name: &str) -> &[String] {
        self.ghost_constraints
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn record_chunk(&mut self, rec: ChunkRecord) {
        self.chunk_records.push(rec);
    }
}
