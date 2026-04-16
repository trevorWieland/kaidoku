use super::environment::{cpu_core_tier, runner_class};
use super::{
    BENCH_SCHEMA_VERSION, BenchBaselineStore, BenchChecks, BenchEnvironment, BenchReport,
    REQUIRED_BASELINE_RUNNER_CLASSES, RuntimeCalibration, bytes_to_mib, load_bench_baseline_store,
    validate_required_runner_classes,
};
use std::path::{Path, PathBuf};

fn phase1_baseline_path() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join("tests")
        .join("golden")
        .join("phase1")
        .join("benchmarks.baseline.json")
}

#[test]
fn bytes_to_mib_handles_values_larger_than_4gib() {
    let five_gib = 5_u64 * 1024 * 1024 * 1024;
    let mib = bytes_to_mib(five_gib);
    assert!(mib > 5_000.0);
    assert!(mib < 5_200.0);
}

#[test]
fn phase1_baseline_contains_all_required_runner_classes() {
    let path = phase1_baseline_path();
    let store = load_bench_baseline_store(&path).expect("baseline should load");
    let present = store
        .reports
        .iter()
        .map(|report| report.environment.runner_class.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for class in REQUIRED_BASELINE_RUNNER_CLASSES {
        assert!(
            present.contains(class),
            "baseline {} is missing required runner class `{}`",
            path.display(),
            class
        );
    }
}

#[test]
fn validate_required_runner_classes_reports_missing_classes() {
    let env = BenchEnvironment {
        runner_class: REQUIRED_BASELINE_RUNNER_CLASSES[0].to_string(),
        ..BenchEnvironment::default()
    };
    let store = BenchBaselineStore {
        schema_version: BENCH_SCHEMA_VERSION.to_string(),
        reports: vec![BenchReport {
            schema_version: BENCH_SCHEMA_VERSION.to_string(),
            environment: env,
            calibration: RuntimeCalibration {
                median_probe_ms: 1.0,
                mad_probe_ms: 0.1,
                probe_ms_samples: Vec::new(),
            },
            fixtures: Vec::new(),
            checks: BenchChecks {
                warmup_iterations: 0,
                throughput_max_regression_ratio: 0.0,
                latency_max_regression_ratio: 0.0,
                throughput_absolute_mib_delta: 0.0,
                full_latency_absolute_ms_slack: 0.0,
                first_page_latency_absolute_ms_slack: 0.0,
                noise_sigma_multiplier: 0.0,
                regression_probability_threshold: 0.0,
                regression_effect_size_floor: 0.0,
            },
        }],
    };
    let err = validate_required_runner_classes(&store, Path::new("baseline.json"))
        .expect_err("missing CI runner classes should error");
    let message = err.to_string();
    for class in &REQUIRED_BASELINE_RUNNER_CLASSES[1..] {
        assert!(
            message.contains(class),
            "error message should list missing class `{class}`, got: {message}"
        );
    }
}

#[test]
fn runner_class_is_stable_and_runner_aware() {
    let class = runner_class("linux", "x86_64", "release", 12, "rustc 1.94.1 (abc)");
    assert!(class.contains("linux-x86_64-release"));
    assert!(class.contains("core-large"));
    assert!(class.contains("rustc1.94"));
    assert_eq!(cpu_core_tier(2), "core-small");
}
