// SPDX-License-Identifier: Apache-2.0
//! ARD (Axiom Recorded Data) — Ultra-Deterministic Forensic Format
//!
//! ARD files capture a 21-step forensic window around a concurrency bug for
//! lossless deterministic replay.  The format is a **hybrid**:
//!
//! - **Header** (JSON-serializable): human-readable metadata, master seed,
//!   Sled snapshot reference.
//! - **Payload** (binary or JSON): the 21-frame forensic trace captured in a
//!   ring-buffer during DPOR exploration.
//!
//! # Window Layout
//!
//! ```text
//! step_index: -10 … -1  (pre-error frames)
//!             0          (error frame)
//!            +1 … +10   (post-error frames)
//! ```
//!
//! # Deterministic Replay
//!
//! Given an `.ard` file, `laplace forensic replay <file>` reloads
//! `axiom_seed` + `snapshot_ref` and re-executes the 21 steps without
//! any deviation.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Window constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Frames captured before the error event.
pub const WINDOW_PRE: usize = 10;

/// Frames captured after the error event.
pub const WINDOW_POST: usize = 10;

/// Total forensic window size (Pre-10 + Error + Post-10).
pub const WINDOW_TOTAL: usize = WINDOW_PRE + 1 + WINDOW_POST;

const ARD_MAGIC: &[u8; 4] = b"LARD";
const ARD_FORMAT_V1_JSON: u8 = 1;
const ARD_FORMAT_V2_POSTCARD: u8 = 2;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ArdHeader — Context Genesis
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Human- and machine-readable context genesis block for a single ARD report.
///
/// Encoded as JSON at the top of every `.ard` file so that tooling can
/// identify and decode the payload without parsing the entire file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArdHeader {
    /// ARD specification version (e.g. `"1.0"`).
    pub version: String,

    /// Master seed governing all randomness during this verification run.
    ///
    /// Storing the seed guarantees that the exact same state-space path can
    /// be re-entered on demand.
    pub axiom_seed: u64,

    /// Crate / module under verification (e.g. `"ticket_counter"`).
    pub target_id: String,

    /// Virtual-clock start point (milliseconds since UNIX epoch).
    pub timestamp_start: i64,

    /// SHA-256 or Sled snapshot hash of the database state at capture time.
    ///
    /// Acts as a rollback anchor: replay loads this snapshot before
    /// re-executing the 21 steps.
    pub snapshot_ref: String,

    /// [Ghost Constraint]: local ARD only. Never include this in Bug DB reports.
    ///
    /// Key: `"R0"`, `"R1"`, `"T0"`, `"T1"`, etc.
    /// Value: human-readable resource/thread/source context.
    #[serde(default)]
    pub symbol_table: HashMap<String, String>,
}

impl ArdHeader {
    /// Construct a new header with `version = "1.0"` and the current wall-clock.
    pub fn new(
        axiom_seed: u64,
        target_id: impl Into<String>,
        snapshot_ref: impl Into<String>,
    ) -> Self {
        Self {
            version: "1.0".to_string(),
            axiom_seed,
            target_id: target_id.into(),
            timestamp_start: crate::domain::now_ms(),
            snapshot_ref: snapshot_ref.into(),
            symbol_table: HashMap::new(),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ForensicFrame — Single Captured Execution Step
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A single captured execution step within the 21-step forensic window.
///
/// `step_index` is relative to the error frame:
/// - `-10` through `-1` → pre-error context
/// - `0` → the exact error event
/// - `+1` through `+10` → post-error observations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForensicFrame {
    /// Position within the window (-10 … 0 … +10).
    pub step_index: i32,

    /// Virtual thread ID that produced this event.
    pub thread_id: String,

    /// Human-readable operation label (e.g. `"FFI Call"`, `"State Write"`,
    /// `"Message Send"`).
    pub operation: String,

    /// Serialized representation of all inputs to this step.
    pub input_dump: String,

    /// Serialized result, or the Panic/Error message if this is the error frame.
    pub output_dump: String,

    /// Logical call-stack path at the time of capture (forensic path).
    ///
    /// Each entry is a human-readable frame description.
    pub stack_trace: Vec<String>,
}

impl ForensicFrame {
    /// Construct a forensic frame.
    pub fn new(
        step_index: i32,
        thread_id: impl Into<String>,
        operation: impl Into<String>,
        input_dump: impl Into<String>,
        output_dump: impl Into<String>,
        stack_trace: Vec<String>,
    ) -> Self {
        Self {
            step_index,
            thread_id: thread_id.into(),
            operation: operation.into(),
            input_dump: input_dump.into(),
            output_dump: output_dump.into(),
            stack_trace,
        }
    }

    /// Convenience constructor for the error (step 0) frame.
    pub fn error_frame(
        thread_id: impl Into<String>,
        operation: impl Into<String>,
        input_dump: impl Into<String>,
        error_message: impl Into<String>,
        stack_trace: Vec<String>,
    ) -> Self {
        Self::new(
            0,
            thread_id,
            operation,
            input_dump,
            error_message,
            stack_trace,
        )
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ForensicWindow — Ring-Buffer Capture Engine
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Ring-buffer that accumulates the 21-step forensic window.
///
/// # Usage pattern
///
/// 1. Call [`push_pre`](Self::push_pre) for every normal execution step.
///    The buffer automatically evicts the oldest entry when it exceeds
///    [`WINDOW_PRE`] frames.
/// 2. Call [`set_error`](Self::set_error) exactly once when the error fires.
/// 3. Call [`push_post`](Self::push_post) for up to [`WINDOW_POST`] steps
///    after the error.
/// 4. Call [`into_report`](Self::into_report) (or [`into_frames`](Self::into_frames))
///    to extract the complete 21-step trace.
#[derive(Debug, Clone, Default)]
pub struct ForensicWindow {
    pre_buffer: VecDeque<ForensicFrame>,
    error_frame: Option<ForensicFrame>,
    post_frames: Vec<ForensicFrame>,
}

impl ForensicWindow {
    /// Create an empty forensic window.
    pub fn new() -> Self {
        Self {
            pre_buffer: VecDeque::with_capacity(WINDOW_PRE),
            error_frame: None,
            post_frames: Vec::with_capacity(WINDOW_POST),
        }
    }

    /// Push a pre-error execution frame.
    ///
    /// Once the buffer exceeds [`WINDOW_PRE`] entries the oldest is dropped
    /// (ring-buffer semantics).  Ignored after [`set_error`](Self::set_error)
    /// has been called.
    pub fn push_pre(&mut self, mut frame: ForensicFrame) {
        if self.error_frame.is_some() {
            return;
        }
        if self.pre_buffer.len() == WINDOW_PRE {
            self.pre_buffer.pop_front();
        }
        // Assign relative step_index (will be corrected in into_frames).
        frame.step_index = -(self.pre_buffer.len() as i32 + 1);
        self.pre_buffer.push_back(frame);
    }

    /// Record the error frame (step index 0).
    ///
    /// Must be called exactly once.  Calling a second time is a no-op.
    pub fn set_error(&mut self, mut frame: ForensicFrame) {
        if self.error_frame.is_none() {
            frame.step_index = 0;
            self.error_frame = Some(frame);
        }
    }

    /// Push a post-error frame.
    ///
    /// At most [`WINDOW_POST`] frames are stored.  Additional calls are
    /// silently ignored.  Has no effect if [`set_error`](Self::set_error) has
    /// not been called yet.
    pub fn push_post(&mut self, mut frame: ForensicFrame) {
        if self.error_frame.is_none() || self.post_frames.len() >= WINDOW_POST {
            return;
        }
        frame.step_index = self.post_frames.len() as i32 + 1;
        self.post_frames.push(frame);
    }

    /// Returns `true` once the error frame has been registered.
    pub fn is_error_set(&self) -> bool {
        self.error_frame.is_some()
    }

    /// Returns `true` when all 21 frames have been collected.
    pub fn is_complete(&self) -> bool {
        self.error_frame.is_some() && self.post_frames.len() == WINDOW_POST
    }

    /// Extract all frames with correct, monotone step indices.
    ///
    /// Pre-frames are renumbered `-N … -1` (oldest first), the error frame
    /// is index `0`, and post-frames are `+1 … +M`.
    pub fn into_frames(self) -> Vec<ForensicFrame> {
        let pre_len = self.pre_buffer.len();
        let mut frames: Vec<ForensicFrame> = self
            .pre_buffer
            .into_iter()
            .enumerate()
            .map(|(i, mut f)| {
                // oldest = most negative index
                f.step_index = (i as i32) - (pre_len as i32);
                f
            })
            .collect();

        if let Some(mut ef) = self.error_frame {
            ef.step_index = 0;
            frames.push(ef);
        }

        for (i, mut pf) in self.post_frames.into_iter().enumerate() {
            pf.step_index = i as i32 + 1;
            frames.push(pf);
        }

        frames
    }

    /// Consume the window and produce a complete [`ArdReport`].
    pub fn into_report(self, header: ArdHeader) -> ArdReport {
        let frames = self.into_frames();
        ArdReport { header, frames }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ArdReport — Complete ARD Package
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A complete ARD (Axiom Recorded Data) report bundling header + 21 forensic frames.
///
/// # Serialization
///
/// Two formats are supported:
///
/// | Method | Format | Use case |
/// |--------|--------|----------|
/// | [`to_json`](Self::to_json) / [`from_json`](Self::from_json) | Pretty JSON | Human inspection, diffs |
/// | [`to_binary`](Self::to_binary) / [`from_binary`](Self::from_binary) | Bincode | Fast I/O, compact storage |
///
/// # File conventions
///
/// ARD files use the `.ard` extension (e.g. `bug_report_20240314_143022.ard`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArdReport {
    /// Metadata block (JSON-readable context genesis).
    pub header: ArdHeader,

    /// Ordered 21-step forensic trace (step_index -10 … 0 … +10).
    pub frames: Vec<ForensicFrame>,
}

#[derive(Debug)]
pub enum ArdFormatError {
    Json(serde_json::Error),
    Postcard(postcard::Error),
    InvalidMagic,
    UnsupportedVersion(u8),
}

impl std::fmt::Display for ArdFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "ARD JSON decode failed: {e}"),
            Self::Postcard(e) => write!(f, "ARD postcard decode failed: {e}"),
            Self::InvalidMagic => write!(f, "ARD magic mismatch"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported ARD version {version}"),
        }
    }
}

impl std::error::Error for ArdFormatError {}

impl ArdReport {
    /// Construct a report directly from a pre-assembled frame list.
    pub fn new(header: ArdHeader, frames: Vec<ForensicFrame>) -> Self {
        Self { header, frames }
    }

    /// Serialize to pretty-printed JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON text.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize to compact binary (postcard format).
    pub fn to_binary(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }

    /// Deserialize from binary (postcard format).
    pub fn from_binary(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }

    /// Serialize to the versioned ARD byte format (`LARD` + v2 + postcard payload).
    pub fn save_to_bytes(&self) -> Result<Vec<u8>, ArdFormatError> {
        let payload = postcard::to_allocvec(self).map_err(ArdFormatError::Postcard)?;
        let mut bytes = Vec::with_capacity(ARD_MAGIC.len() + 1 + payload.len());
        bytes.extend_from_slice(ARD_MAGIC);
        bytes.push(ARD_FORMAT_V2_POSTCARD);
        bytes.extend_from_slice(&payload);
        Ok(bytes)
    }

    /// Decode a versioned ARD byte sequence. v1 JSON and v2 postcard are supported.
    pub fn from_bytes(data: &[u8]) -> Result<Self, ArdFormatError> {
        if data.len() < ARD_MAGIC.len() + 1 || &data[..ARD_MAGIC.len()] != ARD_MAGIC {
            return serde_json::from_slice(data).map_err(ArdFormatError::Json);
        }

        let version = data[ARD_MAGIC.len()];
        let payload = &data[ARD_MAGIC.len() + 1..];
        match version {
            ARD_FORMAT_V1_JSON => serde_json::from_slice(payload).map_err(ArdFormatError::Json),
            ARD_FORMAT_V2_POSTCARD => {
                postcard::from_bytes(payload).map_err(ArdFormatError::Postcard)
            }
            other => Err(ArdFormatError::UnsupportedVersion(other)),
        }
    }

    /// Return the error frame (step_index == 0) if present.
    pub fn error_frame(&self) -> Option<&ForensicFrame> {
        self.frames.iter().find(|f| f.step_index == 0)
    }

    /// Return all pre-error frames (step_index < 0), oldest first.
    pub fn pre_frames(&self) -> impl Iterator<Item = &ForensicFrame> {
        self.frames.iter().filter(|f| f.step_index < 0)
    }

    /// Return all post-error frames (step_index > 0).
    pub fn post_frames(&self) -> impl Iterator<Item = &ForensicFrame> {
        self.frames.iter().filter(|f| f.step_index > 0)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(label: &str) -> ForensicFrame {
        ForensicFrame::new(0, "t0", label, "{}", "{}", vec![])
    }

    #[test]
    fn test_window_pre_ring_buffer() {
        let mut w = ForensicWindow::new();
        // Push 15 pre-frames — only the last 10 should survive.
        for i in 0..15 {
            w.push_pre(make_frame(&format!("op_{i}")));
        }
        assert_eq!(w.pre_buffer.len(), WINDOW_PRE);
        // The oldest surviving frame should be "op_5".
        assert_eq!(w.pre_buffer.front().unwrap().operation, "op_5");
    }

    #[test]
    fn test_window_complete_flow() {
        let mut w = ForensicWindow::new();
        for i in 0..WINDOW_PRE {
            w.push_pre(make_frame(&format!("pre_{i}")));
        }
        w.set_error(ForensicFrame::error_frame(
            "t1",
            "Panic",
            "{}",
            "underflow",
            vec![],
        ));
        for i in 0..WINDOW_POST {
            w.push_post(make_frame(&format!("post_{i}")));
        }

        assert!(w.is_complete());
        let frames = w.into_frames();
        assert_eq!(frames.len(), WINDOW_TOTAL);

        // Verify step indices are monotonically increasing.
        for i in 0..frames.len() - 1 {
            assert!(frames[i].step_index < frames[i + 1].step_index);
        }
        // Error frame is index 0.
        let error = frames.iter().find(|f| f.step_index == 0).unwrap();
        assert_eq!(error.output_dump, "underflow");
    }

    #[test]
    fn test_push_pre_ignored_after_error() {
        let mut w = ForensicWindow::new();
        w.set_error(make_frame("err"));
        w.push_pre(make_frame("ignored"));
        assert_eq!(w.pre_buffer.len(), 0);
    }

    #[test]
    fn test_push_post_cap() {
        let mut w = ForensicWindow::new();
        w.set_error(make_frame("err"));
        for _ in 0..15 {
            w.push_post(make_frame("post"));
        }
        assert_eq!(w.post_frames.len(), WINDOW_POST);
    }

    #[test]
    fn test_json_round_trip() {
        let header = ArdHeader::new(0xDEAD_BEEF, "test_module", "sha256:abc123");
        let mut w = ForensicWindow::new();
        w.set_error(ForensicFrame::error_frame(
            "t0",
            "Deadlock",
            "{}",
            "cycle",
            vec!["frame_a".into()],
        ));
        let report = w.into_report(header);

        let json = report.to_json().unwrap();
        assert!(json.contains("1.0"));
        assert!(json.contains("test_module"));

        let decoded = ArdReport::from_json(&json).unwrap();
        assert_eq!(decoded, report);
    }

    #[test]
    fn test_ard_versioned_bytes_round_trip_v2_postcard() {
        let header = ArdHeader::new(9, "v2_target", "snap:v2");
        let report = ArdReport::new(
            header,
            vec![ForensicFrame::error_frame(
                "t0",
                "Deadlock",
                "{}",
                "cycle",
                vec![],
            )],
        );

        let bytes = report.save_to_bytes().unwrap();
        assert_eq!(&bytes[..4], b"LARD");
        assert_eq!(bytes[4], 2);
        let decoded = ArdReport::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, report);
    }

    #[test]
    fn test_ard_v1_json_compat() {
        let header = ArdHeader::new(10, "v1_target", "snap:v1");
        let report = ArdReport::new(header, vec![]);
        let payload = serde_json::to_vec(&report).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"LARD");
        bytes.push(1);
        bytes.extend_from_slice(&payload);

        let decoded = ArdReport::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, report);
    }

    #[test]
    fn test_binary_round_trip() {
        let header = ArdHeader::new(42, "bin_target", "snap:xyz");
        let report = ArdReport::new(
            header,
            vec![ForensicFrame::error_frame(
                "t0",
                "Panic",
                "{}",
                "boom",
                vec![],
            )],
        );

        let bytes = report.to_binary().unwrap();
        let decoded = ArdReport::from_binary(&bytes).unwrap();
        assert_eq!(decoded, report);
    }
}
