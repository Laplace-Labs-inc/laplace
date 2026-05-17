//! Zero-cost Causality Analysis
//!
//! This module provides tools for analyzing happens-before relationships
//! between simulation events using stack-allocated data structures.

use super::types::{SimulationEvent, ThreadId, MAX_THREADS};
use std::cmp::Ordering;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Happens-before relationship between two events
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HappensBeforeRelation {
    /// Events are causally ordered (e1 happens-before e2)
    Before,
    /// Events are causally ordered (e2 happens-before e1)
    After,
    /// Events are concurrent (no causal relationship)
    Concurrent,
}

impl HappensBeforeRelation {
    /// Determine the relationship between two events based on Lamport timestamps
    ///
    /// **Important**: This is a conservative approximation.
    /// - `timestamp(e1) < timestamp(e2)` implies possible happens-before
    /// - Equal timestamps on different threads → definitely concurrent
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "20_Core_Tracing",
            link = "LEP-0005-laplace-core-tracing_causality"
        )
    )]
    #[inline]
    pub fn from_events(e1: &SimulationEvent, e2: &SimulationEvent) -> Self {
        let meta1 = e1.metadata();
        let meta2 = e2.metadata();

        let ts1 = meta1.timestamp;
        let ts2 = meta2.timestamp;
        let tid1 = meta1.thread_id;
        let tid2 = meta2.thread_id;

        if tid1 == tid2 {
            // Same thread: use sequence number for total ordering
            match meta1.seq_num.cmp(&meta2.seq_num) {
                Ordering::Less => HappensBeforeRelation::Before,
                Ordering::Greater => HappensBeforeRelation::After,
                Ordering::Equal => HappensBeforeRelation::Concurrent, // Should not happen
            }
        } else {
            // Different threads: Lamport clock only gives partial order
            match ts1.0.cmp(&ts2.0) {
                Ordering::Less => HappensBeforeRelation::Before,
                Ordering::Greater => HappensBeforeRelation::After,
                Ordering::Equal => HappensBeforeRelation::Concurrent,
            }
        }
    }
}

/// A graph representing causality relationships between events
///
/// This is a **stack-allocated** structure that builds a happens-before
/// graph from a trace without any heap allocation (except for topological sort).
///
/// # Memory Layout
/// - thread_event_counts: MAX_THREADS * 8 bytes
/// - Total: ~128 bytes (stack-friendly)
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "20_Core_Tracing",
        link = "LEP-0005-laplace-core-tracing_causality"
    )
)]
pub struct CausalityGraph<'a> {
    /// Reference to the trace events
    events: &'a [SimulationEvent],

    /// Number of events per thread (for quick lookup)
    thread_event_counts: [usize; MAX_THREADS],
}

impl<'a> CausalityGraph<'a> {
    /// Build a causality graph from a trace
    ///
    /// This operation is O(n) where n is the number of events,
    /// using only stack-allocated memory.
    pub fn from_trace(events: &'a [SimulationEvent]) -> Self {
        let mut thread_event_counts = [0; MAX_THREADS];

        // Count events per thread
        for event in events {
            let meta = event.metadata();
            let idx = meta.thread_id.as_index();
            if idx < MAX_THREADS {
                thread_event_counts[idx] += 1;
            }
        }

        Self {
            events,
            thread_event_counts,
        }
    }

    /// Check if event at index i happens-before event at index j
    ///
    /// This uses a simplified happens-before check based on Lamport timestamps.
    #[inline]
    pub fn happens_before(&self, i: usize, j: usize) -> bool {
        if i >= self.events.len() || j >= self.events.len() {
            return false;
        }

        let ei = &self.events[i];
        let ej = &self.events[j];

        matches!(
            HappensBeforeRelation::from_events(ei, ej),
            HappensBeforeRelation::Before
        )
    }

    /// Check transitive happens-before (i -> ... -> j)
    ///
    /// This provides a transitive closure of the happens-before relation.
    pub fn happens_before_transitive(&self, i: usize, j: usize) -> bool {
        if i == j || i >= self.events.len() || j >= self.events.len() {
            return false;
        }

        let meta_i = self.events[i].metadata();
        let meta_j = self.events[j].metadata();

        // For same thread, use sequence number
        if meta_i.thread_id == meta_j.thread_id {
            return meta_i.seq_num < meta_j.seq_num;
        }

        // For different threads, strictly less timestamp implies happens-before
        meta_i.timestamp < meta_j.timestamp
    }

    /// Find all events that are concurrent with the given event
    ///
    /// Returns indices of concurrent events.
    pub fn find_concurrent(&self, event_idx: usize) -> impl Iterator<Item = usize> + '_ {
        (0..self.events.len()).filter(move |&idx| {
            idx != event_idx
                && !self.happens_before(event_idx, idx)
                && !self.happens_before(idx, event_idx)
        })
    }

    /// Get events in topological order (respecting happens-before)
    ///
    /// This returns event indices sorted by Lamport timestamp.
    pub fn topological_order(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.events.len()).collect();

        // Sort by Lamport timestamp
        indices.sort_by_key(|&i| self.events[i].metadata().timestamp.0);

        indices
    }

    /// Get the number of events in the graph
    #[inline(always)]
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Get events for a specific thread in program order
    pub fn thread_events(&self, thread_id: ThreadId) -> impl Iterator<Item = usize> + '_ {
        self.events
            .iter()
            .enumerate()
            .filter(move |(_, e)| e.metadata().thread_id == thread_id)
            .map(|(i, _)| i)
    }

    /// Get the number of events from a specific thread
    #[inline]
    pub fn thread_event_count(&self, thread_id: ThreadId) -> usize {
        let idx = thread_id.as_index();
        if idx < MAX_THREADS {
            self.thread_event_counts[idx]
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::memory::Address;
    use crate::domain::tracing::types::{EventMetadata, LamportTimestamp, MemoryOperation};

    fn create_dummy_event(ts: u64, tid: u32, seq: u64) -> SimulationEvent {
        let meta = EventMetadata::new(LamportTimestamp(ts), ThreadId::new(tid), seq);
        SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Read {
                addr: Address::new(0),
                value: 0,
                cache_hit: false,
            },
        }
    }

    #[test]
    fn test_happens_before_same_thread() {
        let e1 = create_dummy_event(1, 0, 0);
        let e2 = create_dummy_event(2, 0, 1);

        assert_eq!(
            HappensBeforeRelation::from_events(&e1, &e2),
            HappensBeforeRelation::Before
        );
        assert_eq!(
            HappensBeforeRelation::from_events(&e2, &e1),
            HappensBeforeRelation::After
        );
    }

    #[test]
    fn test_causality_graph_construction() {
        let events = vec![create_dummy_event(1, 0, 0), create_dummy_event(2, 0, 1)];

        let graph = CausalityGraph::from_trace(&events);

        assert_eq!(graph.event_count(), 2);
        assert!(graph.happens_before(0, 1));
        assert!(!graph.happens_before(1, 0));
    }

    #[test]
    fn test_thread_event_count() {
        let events = vec![
            create_dummy_event(1, 0, 0),
            create_dummy_event(2, 1, 0),
            create_dummy_event(3, 0, 1),
        ];

        let graph = CausalityGraph::from_trace(&events);

        assert_eq!(graph.thread_event_count(ThreadId::new(0)), 2);
        assert_eq!(graph.thread_event_count(ThreadId::new(1)), 1);
    }
}
