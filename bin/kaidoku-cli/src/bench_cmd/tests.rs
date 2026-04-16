use super::environment::{compat_class, cpu_core_tier, runner_class};
use super::{
    BENCH_SCHEMA_VERSION, BenchBaselineStore, BenchChecks, BenchEnvironment, BenchReport,
    REQUIRED_BASELINE_RUNNER_CLASSES, RuntimeCalibration, bytes_to_mib,
    load_baseline_with_fallback, load_bench_baseline_store, validate_required_runner_classes,
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

#[test]
fn compat_class_is_os_arch_profile_only() {
    let class = compat_class("macos", "aarch64", "release");
    assert_eq!(class, "macos-aarch64-release");
}

#[test]
fn compat_fallback_selects_loose_match_when_strict_missing() {
    let store = store_with_classes(
        "linux-x86_64-release-core-medium-rustc1.90",
        "linux-x86_64-release",
    );
    let path = temp_baseline(&store);
    let (report, kind) = load_baseline_with_fallback(
        &path,
        "linux-x86_64-release-core-large-rustc1.94",
        "linux-x86_64-release",
        false,
    )
    .expect("compat fallback");
    assert!(kind.starts_with("compat:"));
    assert_eq!(report.environment.compat_class, "linux-x86_64-release");
    std::fs::remove_file(path).ok();
}

#[test]
fn strict_mode_rejects_compat_fallback() {
    let store = store_with_classes(
        "linux-x86_64-release-core-medium-rustc1.90",
        "linux-x86_64-release",
    );
    let path = temp_baseline(&store);
    let err = load_baseline_with_fallback(
        &path,
        "linux-x86_64-release-core-large-rustc1.94",
        "linux-x86_64-release",
        true,
    )
    .expect_err("strict mode rejects compat fallback");
    std::fs::remove_file(path).ok();
    assert!(err.to_string().contains("strict mode"));
}

#[test]
fn strict_match_wins_over_compat_match_when_both_present() {
    let store = store_with_classes(
        "linux-x86_64-release-core-large-rustc1.94",
        "linux-x86_64-release",
    );
    let path = temp_baseline(&store);
    let (_, kind) = load_baseline_with_fallback(
        &path,
        "linux-x86_64-release-core-large-rustc1.94",
        "linux-x86_64-release",
        false,
    )
    .expect("strict match wins");
    std::fs::remove_file(path).ok();
    assert!(kind.starts_with("strict:"));
}

fn store_with_classes(runner: &str, compat: &str) -> BenchBaselineStore {
    // Build a store that (a) satisfies the required-classes validator for
    // every CI-blocking runner and (b) ADDS a custom (runner, compat) entry
    // that the fallback logic can observe. Overwriting one of the required
    // entries would trip validation before the fallback check runs.
    let mut reports = REQUIRED_BASELINE_RUNNER_CLASSES
        .iter()
        .map(|class| make_sample_report(class, &compat_from_runner_class(class)))
        .collect::<Vec<_>>();
    reports.push(make_sample_report(runner, compat));
    BenchBaselineStore {
        schema_version: BENCH_SCHEMA_VERSION.to_string(),
        reports,
    }
}

fn make_sample_report(runner: &str, compat: &str) -> BenchReport {
    BenchReport {
        schema_version: BENCH_SCHEMA_VERSION.to_string(),
        environment: BenchEnvironment {
            runner_class: runner.to_string(),
            compat_class: compat.to_string(),
            ..BenchEnvironment::default()
        },
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
    }
}

fn compat_from_runner_class(runner: &str) -> String {
    runner.split('-').take(3).collect::<Vec<_>>().join("-")
}

fn temp_baseline(store: &BenchBaselineStore) -> PathBuf {
    let path = std::env::temp_dir().join(format!("kaidoku_bench_test_{}.json", std::process::id()));
    let json = serde_json::to_string_pretty(store).expect("serialize");
    std::fs::write(&path, json).expect("write temp baseline");
    path
}
