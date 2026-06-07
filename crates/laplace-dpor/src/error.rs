// SPDX-License-Identifier: Apache-2.0
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaplaceError {
    ResourceLimit {
        resource: &'static str,
        limit: usize,
        requested: usize,
    },
}

impl fmt::Display for LaplaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResourceLimit {
                resource,
                limit,
                requested,
            } => write!(
                f,
                "resource limit exceeded for {resource}: requested {requested}, limit {limit}"
            ),
        }
    }
}

impl std::error::Error for LaplaceError {}
