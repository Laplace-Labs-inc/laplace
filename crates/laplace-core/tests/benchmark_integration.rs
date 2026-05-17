//! Integration test for Phase 4.2 CPI Engine

use laplace_core::domain::{CPICalculator, EfficiencyTier, MockResourceMonitor};

#[test]
fn test_cpi_engine_basic_flow() {
    // Create a mock resource monitor
    let monitor = MockResourceMonitor::new();

    // Set some resource values
    monitor.set_cpu_percent(50.0);
    monitor.set_memory_mb(1024.0);

    // Create calculator
    let calculator = CPICalculator::new(Box::new(monitor));

    // Evaluate efficiency
    let (cpi, tier) = calculator.evaluate();

    // Verify values are sensible
    assert!(cpi >= 0.0, "CPI should be non-negative");
    assert_eq!(
        tier,
        EfficiencyTier::Bronze,
        "With 50% CPU and 1GB RAM, should be Bronze"
    );
}

#[test]
fn test_efficiency_tier_classification() {
    let tiers = vec![
        (500.0, EfficiencyTier::Bronze),
        (1500.0, EfficiencyTier::Silver),
        (7000.0, EfficiencyTier::Gold),
        (12000.0, EfficiencyTier::Turbo),
    ];

    for (cpi, expected_tier) in tiers {
        let actual_tier = EfficiencyTier::from_cpi(cpi);
        assert_eq!(
            actual_tier, expected_tier,
            "CPI {} should map to {}",
            cpi, expected_tier
        );
    }
}

#[test]
fn test_mock_monitor_concurrent_updates() {
    let monitor = MockResourceMonitor::new();

    // Test thread-safe updates
    let monitor_clone = monitor.clone();
    std::thread::spawn(move || {
        monitor_clone.set_cpu_percent(75.0);
        monitor_clone.set_memory_mb(2048.0);
    })
    .join()
    .unwrap();

    // Verify updates persisted
    assert_eq!(monitor.get_cpu_percent(), 75.0);
    assert_eq!(monitor.get_memory_mb(), 2048.0);
}

#[test]
fn test_efficiency_tier_display() {
    assert_eq!(EfficiencyTier::Bronze.to_string(), "Bronze");
    assert_eq!(EfficiencyTier::Silver.to_string(), "Silver");
    assert_eq!(EfficiencyTier::Gold.to_string(), "Gold");
    assert_eq!(EfficiencyTier::Turbo.to_string(), "Turbo");
}

#[test]
fn test_cpi_report_generation() {
    let monitor = MockResourceMonitor::new();
    monitor.set_cpu_percent(30.0);
    monitor.set_memory_mb(768.0);

    let calculator = CPICalculator::new(Box::new(monitor));
    let report = calculator.report();

    // Verify report contains key information
    assert!(report.contains("CPI Benchmark Report"));
    assert!(report.contains("CPU Usage"));
    assert!(report.contains("Memory"));
    assert!(report.contains("Tier"));
}
