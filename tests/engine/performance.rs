// Copyright:
//   - Copyright © 2026 Alberto Villa Osorno.
// SPDX-License-Identifier:
//   - Apache-2.0
// Confidential:
//   - false
// License-File:
//   - LICENSE
//
// Boundary-Contract:
// - Owns:
//   - Synthetic measurement aggregation and quality profile invariants.
// - Must-Not:
//   - Report synthetic fixtures as real game or GPU benchmark results.
// - Allows:
//   - Inputs: Deterministic frame durations, latency and byte budgets.
//   - Outputs: Public throughput, peak-memory and overflow assertions.
//   - Side effects: None outside in-memory collector state.
// - Split-When:
//   - Hardware observations require platform-specific benchmark suites.
// - Merge-When:
//   - Shared performance fixtures cover the same properties.
// - Summary:
//   - Portable instrumented performance calculation tests.
// - Description:
//   - Validates calculations, budget summaries and profile isolation.
// - Usage:
//   - cargo validate
// - Defaults:
//   - Synthetic numbers must not be used as hardware performance claims.
//

use std::time::Duration;

use super::{FrameObservation, PerformanceCollector, QualityProfile};

fn profile() -> QualityProfile {
    QualityProfile {
        height: 720,
        optional_effects: false,
        width: 1280,
    }
}

#[test]
fn reports_real_units_from_supplied_frame_observations() {
    let mut collector = PerformanceCollector::new(profile());
    assert_eq!(collector.frames_per_second(), None);
    assert_eq!(
        collector.observe(FrameObservation {
            frame_time: Duration::from_millis(10),
            input_latency: Duration::from_millis(2),
            resident_bytes: 1_000_000,
        }),
        Ok(())
    );
    assert_eq!(
        collector.observe(FrameObservation {
            frame_time: Duration::from_millis(30),
            input_latency: Duration::from_millis(15),
            resident_bytes: 2_000_000,
        }),
        Ok(())
    );
    let report = collector.report();
    assert_eq!(report.frames, 2);
    assert_eq!(report.total_frame_time, Duration::from_millis(40));
    assert_eq!(report.slowest_frame, Duration::from_millis(30));
    assert_eq!(report.max_input_latency, Duration::from_millis(15));
    assert_eq!(report.peak_resident_bytes, 2_000_000);
    assert_eq!(collector.frames_per_second(), Some(50));
}

#[test]
fn records_platform_load_size_and_quality_without_hidden_changes() {
    let mut collector = PerformanceCollector::new(profile());
    collector.record_load(Duration::from_millis(450));
    collector.record_binary_size(123_456);
    assert_eq!(collector.load_duration(), Some(Duration::from_millis(450)));
    assert_eq!(collector.binary_size(), Some(123_456));
    assert_eq!(collector.profile(), profile());
}

#[test]
fn rejects_cumulative_duration_overflow_without_partial_changes() {
    let mut collector = PerformanceCollector::new(profile());
    let first = FrameObservation {
        frame_time: Duration::MAX,
        input_latency: Duration::ZERO,
        resident_bytes: 1,
    };
    assert_eq!(collector.observe(first), Ok(()));
    let before = collector.report();
    let second = FrameObservation {
        frame_time: Duration::from_nanos(1),
        input_latency: Duration::from_secs(9),
        resident_bytes: 100,
    };
    assert!(collector.observe(second).is_err());
    assert_eq!(collector.report(), before);
}
