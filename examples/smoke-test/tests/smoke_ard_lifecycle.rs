#![deny(clippy::all, clippy::pedantic)]

//! Smoke Test: ARD forensic file lifecycle (generation, load, validation).
//!
//! Tests the complete ARD workflow:
//! 1. Bug-producing harness execution
//! 2. ARD file generation (write_ard=true)
//! 3. File existence and validity
//! 4. ArdReport JSON load round-trip
//! 5. JSON/binary serialization

use laplace_core::domain::journal::ard::ArdReport;
use smoke_test::{run_harness_smoke, verdict_matches_expected, OracleVerdict};
use tempfile::TempDir;

fn load_ard_json(path: &str) -> ArdReport {
    let content = std::fs::read_to_string(path).expect("Failed to read ARD file");
    ArdReport::from_json(&content).expect("Failed to parse ARD JSON")
}

/// Generate ARD file from a bug-producing harness.
///
/// Runs resource_abba_deadlock with write_ard=true and verifies:
/// 1. Verdict is BugFound
/// 2. ARD file exists
/// 3. File has non-zero size
#[test]
fn smoke_ard_generation_on_bug_found() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let output_dir = temp_dir.path().to_str().expect("Invalid path");

    let verdict = run_harness_smoke("resource_abba_deadlock", 10000, true, output_dir);

    assert!(
        matches!(verdict, OracleVerdict::BugFound { .. }),
        "resource_abba_deadlock must find bug, got {:?}",
        verdict
    );

    // ARD file should be created in output_dir
    // File name pattern: {harness_name}_ard.bin or similar
    let ard_files: Vec<_> = std::fs::read_dir(output_dir)
        .expect("Failed to read output directory")
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "ard") {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    assert!(
        !ard_files.is_empty(),
        "ARD file should be generated in output directory"
    );

    let ard_path = &ard_files[0];
    let metadata = std::fs::metadata(ard_path).expect("Failed to get file metadata");
    assert!(metadata.len() > 0, "ARD file should not be empty");
}

/// Load and validate ARD report structure.
///
/// Verifies:
/// 1. ArdReport JSON load succeeds
/// 2. Header fields are populated correctly
/// 3. Frame count is valid
/// 4. Error frame exists and is correct
#[test]
fn smoke_ard_load_and_structure_verification() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let output_dir = temp_dir.path().to_str().expect("Invalid path");

    let verdict = run_harness_smoke("resource_abba_deadlock", 10000, true, output_dir);

    assert!(
        matches!(verdict, OracleVerdict::BugFound { .. }),
        "resource_abba_deadlock must find bug"
    );

    // Find .ard file
    let ard_files: Vec<_> = std::fs::read_dir(output_dir)
        .expect("Failed to read output directory")
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "ard") {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    assert!(!ard_files.is_empty(), "ARD file should exist");

    let ard_path = ard_files[0].to_str().expect("Invalid path");
    let report = load_ard_json(ard_path);

    // Verify header
    assert_eq!(report.header.version, "1.0", "ARD version should be 1.0");
    assert_eq!(
        report.header.target_id, "harness::resource_abba_deadlock",
        "Target ID should match harness name"
    );
    assert_ne!(
        report.header.axiom_seed, 0,
        "Axiom seed should be initialized"
    );

    // Verify frames
    assert!(
        !report.frames.is_empty(),
        "ARD should contain at least one frame"
    );
    assert!(
        report.frames.len() <= 21,
        "ARD frames should not exceed forensic window size (21)"
    );

    // Verify error frame
    let error_frame = report.error_frame();
    assert!(
        error_frame.is_some(),
        "Error frame should exist in bug report"
    );
    if let Some(frame) = error_frame {
        // Error frame should be at step 0 (most recent in the trace)
        assert_eq!(frame.step_index, 0, "Error frame should be at step index 0");
    }
}

/// Clean harness should not generate ARD file.
///
/// Verifies:
/// 1. Running a clean harness with write_ard=true returns Clean
/// 2. No ARD file is created
#[test]
fn smoke_ard_not_generated_on_clean() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let output_dir = temp_dir.path().to_str().expect("Invalid path");

    let verdict = run_harness_smoke("template_harness", 10000, true, output_dir);

    assert!(
        verdict_matches_expected(&verdict, "clean"),
        "template_harness should be clean"
    );

    // Verify no ARD file was created
    let ard_files: Vec<_> = std::fs::read_dir(output_dir)
        .expect("Failed to read output directory")
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "ard") {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    assert!(
        ard_files.is_empty(),
        "No ARD file should be generated for clean harness"
    );
}

/// Verify ARD JSON round-trip serialization.
///
/// Verifies:
/// 1. to_json() succeeds
/// 2. from_json() succeeds
/// 3. Round-trip preserves structure
#[test]
fn smoke_ard_json_roundtrip() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let output_dir = temp_dir.path().to_str().expect("Invalid path");

    let verdict = run_harness_smoke("resource_abba_deadlock", 10000, true, output_dir);

    assert!(matches!(verdict, OracleVerdict::BugFound { .. }));

    // Load original report
    let ard_files: Vec<_> = std::fs::read_dir(output_dir)
        .expect("Failed to read output directory")
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "ard") {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    let ard_path = ard_files[0].to_str().expect("Invalid path");
    let original = load_ard_json(ard_path);

    // Serialize to JSON and back
    let json = original.to_json().expect("Failed to serialize ARD to JSON");
    let restored = ArdReport::from_json(&json).expect("Failed to deserialize ARD from JSON");

    // Verify structure is preserved
    assert_eq!(
        original.header.version, restored.header.version,
        "Version should match after round-trip"
    );
    assert_eq!(
        original.header.target_id, restored.header.target_id,
        "Target ID should match after round-trip"
    );
    assert_eq!(
        original.frames.len(),
        restored.frames.len(),
        "Frame count should match after round-trip"
    );
}
