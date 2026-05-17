//! Vector Clock Implementation
//!
//! Vector clocks are used to track causality (happens-before relation)
//! in concurrent executions.
//!
//! # Theory
//!
//! Given events e1 and e2:
//! - e1 happens-before e2 (e1 → e2) if VC(e1) < VC(e2)
//! - e1 and e2 are concurrent if neither happens-before the other
//!
//! # Implementation
//!
//! Uses fixed-size array for zero heap allocation and cache efficiency.

use super::MAX_THREADS;
use laplace_interfaces::domain::resource::ThreadId;
use std::fmt;

/// Vector clock for tracking causality
///
/// # Representation
///
/// Each thread maintains a vector of logical clocks, one entry per thread.
/// - `clock[i]` = thread i's view of its own logical time
/// - `clock[j]` = thread i's view of thread j's logical time
///
/// # Example
///
/// ```text
/// Thread 0: [5, 2, 3]  (T0 has executed 5 ops, last saw T1 at 2, T2 at 3)
/// Thread 1: [4, 7, 3]  (T1 has executed 7 ops, last saw T0 at 4, T2 at 3)
/// Thread 2: [5, 6, 8]  (T2 has executed 8 ops, last saw T0 at 5, T1 at 6)
/// ```
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VectorClock {
    /// Clock values for each thread
    clocks: [u64; MAX_THREADS],
}

impl VectorClock {
    /// Create a new vector clock initialized to zero
    #[inline]
    pub fn new() -> Self {
        Self {
            clocks: [0; MAX_THREADS],
        }
    }

    /// Increment the clock for a specific thread
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut vc = VectorClock::new();
    /// vc.tick(ThreadId(0));  // [1, 0, 0, ...]
    /// vc.tick(ThreadId(0));  // [2, 0, 0, ...]
    /// ```
    #[inline]
    pub fn tick(&mut self, thread: ThreadId) {
        let idx = thread.as_usize();
        if idx < MAX_THREADS {
            self.clocks[idx] = self.clocks[idx].saturating_add(1);
        }
    }

    /// Merge with another vector clock (element-wise max)
    ///
    /// This is used when a thread receives a message from another thread.
    ///
    /// # Example
    ///
    /// ```text
    /// self:  [3, 1, 2]
    /// other: [2, 5, 1]
    /// result:[3, 5, 2]  (max of each position)
    /// ```
    #[inline]
    pub fn merge(&mut self, other: &VectorClock) {
        for i in 0..MAX_THREADS {
            self.clocks[i] = self.clocks[i].max(other.clocks[i]);
        }
    }

    /// Check if this clock happens-before another
    ///
    /// # Definition
    ///
    /// VC1 < VC2 iff:
    /// - For all i: VC1[i] <= VC2[i], AND
    /// - Exists j: VC1[j] < VC2[j]
    ///
    /// # Example
    ///
    /// ```text
    /// [1, 2, 3] happens-before [2, 3, 4]  ✓
    /// [1, 2, 3] happens-before [1, 2, 3]  ✗ (equal, not strictly before)
    /// [1, 3, 3] happens-before [2, 2, 4]  ✗ (1 < 2 but 3 > 2)
    /// ```
    pub fn happens_before(&self, other: &VectorClock) -> bool {
        let mut all_less_or_equal = true;
        let mut some_strictly_less = false;

        for i in 0..MAX_THREADS {
            if self.clocks[i] > other.clocks[i] {
                all_less_or_equal = false;
                break;
            }
            if self.clocks[i] < other.clocks[i] {
                some_strictly_less = true;
            }
        }

        all_less_or_equal && some_strictly_less
    }

    /// Check if two clocks are concurrent (neither happens-before the other)
    ///
    /// # Example
    ///
    /// ```text
    /// [1, 3, 2] and [2, 1, 3] are concurrent
    /// (neither is strictly less than the other)
    /// ```
    #[inline]
    pub fn concurrent(&self, other: &VectorClock) -> bool {
        !self.happens_before(other) && !other.happens_before(self)
    }

    /// Get clock value for a specific thread
    #[inline]
    pub fn get(&self, thread: ThreadId) -> u64 {
        let idx = thread.as_usize();
        if idx < MAX_THREADS {
            self.clocks[idx]
        } else {
            0
        }
    }

    /// Set clock value for a specific thread (used in testing)
    #[inline]
    pub fn set(&mut self, thread: ThreadId, value: u64) {
        let idx = thread.as_usize();
        if idx < MAX_THREADS {
            self.clocks[idx] = value;
        }
    }
}

impl Default for VectorClock {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for VectorClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VC[")?;
        for (i, &val) in self.clocks.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", val)?;
        }
        write!(f, "]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_clock_init() {
        let vc = VectorClock::new();
        for i in 0..MAX_THREADS {
            assert_eq!(vc.clocks[i], 0);
        }
    }

    #[test]
    fn test_tick() {
        let mut vc = VectorClock::new();
        vc.tick(ThreadId(0));
        assert_eq!(vc.get(ThreadId(0)), 1);
        assert_eq!(vc.get(ThreadId(1)), 0);

        vc.tick(ThreadId(0));
        assert_eq!(vc.get(ThreadId(0)), 2);
    }

    #[test]
    fn test_merge() {
        let mut vc1 = VectorClock::new();
        vc1.set(ThreadId(0), 3);
        vc1.set(ThreadId(1), 1);
        vc1.set(ThreadId(2), 2);

        let mut vc2 = VectorClock::new();
        vc2.set(ThreadId(0), 2);
        vc2.set(ThreadId(1), 5);
        vc2.set(ThreadId(2), 1);

        vc1.merge(&vc2);

        assert_eq!(vc1.get(ThreadId(0)), 3);
        assert_eq!(vc1.get(ThreadId(1)), 5);
        assert_eq!(vc1.get(ThreadId(2)), 2);
    }

    #[test]
    fn test_happens_before() {
        let mut vc1 = VectorClock::new();
        vc1.set(ThreadId(0), 1);
        vc1.set(ThreadId(1), 2);
        vc1.set(ThreadId(2), 3);

        let mut vc2 = VectorClock::new();
        vc2.set(ThreadId(0), 2);
        vc2.set(ThreadId(1), 3);
        vc2.set(ThreadId(2), 4);

        assert!(vc1.happens_before(&vc2));
        assert!(!vc2.happens_before(&vc1));
    }

    #[test]
    fn test_concurrent() {
        let mut vc1 = VectorClock::new();
        vc1.set(ThreadId(0), 1);
        vc1.set(ThreadId(1), 3);
        vc1.set(ThreadId(2), 2);

        let mut vc2 = VectorClock::new();
        vc2.set(ThreadId(0), 2);
        vc2.set(ThreadId(1), 1);
        vc2.set(ThreadId(2), 3);

        assert!(vc1.concurrent(&vc2));
        assert!(vc2.concurrent(&vc1));
    }

    #[test]
    fn test_not_concurrent_when_happens_before() {
        let mut vc1 = VectorClock::new();
        vc1.set(ThreadId(0), 1);

        let mut vc2 = VectorClock::new();
        vc2.set(ThreadId(0), 2);

        assert!(!vc1.concurrent(&vc2));
    }
}
