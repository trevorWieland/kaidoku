use super::{
    BenchReport, FIRST_PAGE_LATENCY_ABSOLUTE_MS_SLACK, FULL_LATENCY_ABSOLUTE_MS_SLACK,
    FixtureBenchResult, LATENCY_MAX_REGRESSION_RATIO, NOISE_SIGMA_MULTIPLIER,
    REGRESSION_EFFECT_SIZE_FLOOR, REGRESSION_PROBABILITY_THRESHOLD, RuntimeCalibration,
    THROUGHPUT_ABSOLUTE_MIB_DELTA, THROUGHPUT_MAX_REGRESSION_RATIO,
};
use anyhow::{Result, bail};
use std::collections::{BTreeMap, BTreeSet};

const MIN_BYTES_FOR_STRICT_LATENCY_GATES: u64 = 16 * 1024;

pub(super) fn check_bench_regression(baseline: &BenchReport, current: &BenchReport) -> Result<()> {
    ensure_same_fixture_set(baseline, current)?;
    let effective_scale = runtime_scale_factor(&baseline.calibration, &current.calibration);
    let baseline_map: BTreeMap<&str, &FixtureBenchResult> = baseline
        .fixtures
        .iter()
        .map(|fixture| (fixture.fixture.as_str(), fixture))
        .collect();

    for fixture in &current.fixtures {
        let Some(previous) = baseline_map.get(fixture.fixture.as_str()) else {
            bail!(
                "fixture {} is missing from benchmark baseline",
                fixture.fixture
            );
        };
        check_fixture_regression(previous, fixture, effective_scale)?;
    }

    Ok(())
}

fn ensure_same_fixture_set(baseline: &BenchReport, current: &BenchReport) -> Result<()> {
    let baseline_names = baseline
        .fixtures
        .iter()
        .map(|fixture| fixture.fixture.as_str())
        .collect::<BTreeSet<_>>();
    let current_names = current
        .fixtures
        .iter()
        .map(|fixture| fixture.fixture.as_str())
        .collect::<BTreeSet<_>>();

    if baseline_names != current_names {
        let missing = baseline_names
            .difference(&current_names)
            .copied()
            .collect::<Vec<_>>();
        let extra = current_names
            .difference(&baseline_names)
            .copied()
            .collect::<Vec<_>>();
        bail!(
            "benchmark fixture-set mismatch between baseline and current; missing: [{}], extra: [{}]",
            missing.join(", "),
            extra.join(", ")
        );
    }

    Ok(())
}

fn check_fixture_regression(
    baseline_fixture: &FixtureBenchResult,
    current_fixture: &FixtureBenchResult,
    runtime_scale: f64,
) -> Result<()> {
    if baseline_fixture.bytes < MIN_BYTES_FOR_STRICT_LATENCY_GATES
        || current_fixture.bytes < MIN_BYTES_FOR_STRICT_LATENCY_GATES
    {
        check_throughput_regression(baseline_fixture, current_fixture, runtime_scale)?;
        return Ok(());
    }

    check_throughput_regression(baseline_fixture, current_fixture, runtime_scale)?;
    check_full_latency_regression(baseline_fixture, current_fixture, runtime_scale)?;
    check_first_page_latency_regression(baseline_fixture, current_fixture, runtime_scale)?;
    Ok(())
}

fn check_throughput_regression(
    baseline_fixture: &FixtureBenchResult,
    current_fixture: &FixtureBenchResult,
    runtime_scale: f64,
) -> Result<()> {
    let adjusted_current_median =
        scale_throughput(current_fixture.median_throughput_mib_per_s, runtime_scale);
    let adjusted_current_mad =
        scale_throughput(current_fixture.mad_throughput_mib_per_s, runtime_scale);
    let adjusted_current_samples = current_fixture
        .throughput_mib_per_s_samples
        .iter()
        .map(|sample| scale_throughput(*sample, runtime_scale))
        .collect::<Vec<_>>();

    let drop = regression_ratio(
        baseline_fixture.median_throughput_mib_per_s,
        adjusted_current_median,
        true,
    );
    let limit = threshold_with_noise(
        THROUGHPUT_MAX_REGRESSION_RATIO,
        baseline_fixture.median_throughput_mib_per_s,
        baseline_fixture.mad_throughput_mib_per_s,
        adjusted_current_mad,
    );
    let probability = pairwise_regression_probability(
        &baseline_fixture.throughput_mib_per_s_samples,
        &adjusted_current_samples,
        true,
    );
    let throughput_noise_tolerance = (NOISE_SIGMA_MULTIPLIER
        * (baseline_fixture.mad_throughput_mib_per_s + adjusted_current_mad))
        .max(THROUGHPUT_ABSOLUTE_MIB_DELTA);
    let absolute_floor =
        (baseline_fixture.median_throughput_mib_per_s - throughput_noise_tolerance).max(0.0);
    if adjusted_current_median < absolute_floor {
        bail!(
            "throughput absolute floor breach for {}: {:.3} MiB/s < {:.3} MiB/s (tolerance {:.3} MiB/s)",
            current_fixture.fixture,
            adjusted_current_median,
            absolute_floor,
            throughput_noise_tolerance,
        );
    }
    if drop > limit + REGRESSION_EFFECT_SIZE_FLOOR && probability > REGRESSION_PROBABILITY_THRESHOLD
    {
        bail!(
            "throughput regression for {} is {:.3}, exceeds {:.3} with one-sided probability {:.3}",
            current_fixture.fixture,
            drop,
            limit,
            probability,
        );
    }
    Ok(())
}

fn check_full_latency_regression(
    baseline_fixture: &FixtureBenchResult,
    current_fixture: &FixtureBenchResult,
    runtime_scale: f64,
) -> Result<()> {
    let adjusted_current_median = scale_latency(current_fixture.median_full_ms, runtime_scale);
    let adjusted_current_mad = scale_latency(current_fixture.mad_full_ms, runtime_scale);
    let adjusted_current_samples = current_fixture
        .full_ms_samples
        .iter()
        .map(|sample| scale_latency(*sample, runtime_scale))
        .collect::<Vec<_>>();

    let increase = regression_ratio(
        baseline_fixture.median_full_ms,
        adjusted_current_median,
        false,
    );
    let limit = threshold_with_noise(
        LATENCY_MAX_REGRESSION_RATIO,
        baseline_fixture.median_full_ms,
        baseline_fixture.mad_full_ms,
        adjusted_current_mad,
    );
    let probability = pairwise_regression_probability(
        &baseline_fixture.full_ms_samples,
        &adjusted_current_samples,
        false,
    );
    let absolute_cap = baseline_fixture.median_full_ms + FULL_LATENCY_ABSOLUTE_MS_SLACK;
    if adjusted_current_median > absolute_cap {
        bail!(
            "full extraction latency absolute cap breach for {}: {:.3} ms > {:.3} ms",
            current_fixture.fixture,
            adjusted_current_median,
            absolute_cap,
        );
    }
    if increase > limit + REGRESSION_EFFECT_SIZE_FLOOR
        && probability > REGRESSION_PROBABILITY_THRESHOLD
    {
        bail!(
            "full extraction latency regression for {} is {:.3}, exceeds {:.3} with one-sided probability {:.3}",
            current_fixture.fixture,
            increase,
            limit,
            probability,
        );
    }
    Ok(())
}

fn check_first_page_latency_regression(
    baseline_fixture: &FixtureBenchResult,
    current_fixture: &FixtureBenchResult,
    runtime_scale: f64,
) -> Result<()> {
    let adjusted_current_median =
        scale_latency(current_fixture.median_first_page_ms, runtime_scale);
    let adjusted_current_mad = scale_latency(current_fixture.mad_first_page_ms, runtime_scale);
    let adjusted_current_samples = current_fixture
        .first_page_ms_samples
        .iter()
        .map(|sample| scale_latency(*sample, runtime_scale))
        .collect::<Vec<_>>();

    let increase = regression_ratio(
        baseline_fixture.median_first_page_ms,
        adjusted_current_median,
        false,
    );
    let limit = threshold_with_noise(
        LATENCY_MAX_REGRESSION_RATIO,
        baseline_fixture.median_first_page_ms,
        baseline_fixture.mad_first_page_ms,
        adjusted_current_mad,
    );
    let probability = pairwise_regression_probability(
        &baseline_fixture.first_page_ms_samples,
        &adjusted_current_samples,
        false,
    );
    let absolute_cap = baseline_fixture.median_first_page_ms + FIRST_PAGE_LATENCY_ABSOLUTE_MS_SLACK;
    if adjusted_current_median > absolute_cap {
        bail!(
            "first-page latency absolute cap breach for {}: {:.3} ms > {:.3} ms",
            current_fixture.fixture,
            adjusted_current_median,
            absolute_cap,
        );
    }
    if increase > limit + REGRESSION_EFFECT_SIZE_FLOOR
        && probability > REGRESSION_PROBABILITY_THRESHOLD
    {
        bail!(
            "first-page latency regression for {} is {:.3}, exceeds {:.3} with one-sided probability {:.3}",
            current_fixture.fixture,
            increase,
            limit,
            probability,
        );
    }

    Ok(())
}

fn threshold_with_noise(
    base: f64,
    baseline_median: f64,
    baseline_mad: f64,
    current_mad: f64,
) -> f64 {
    if baseline_median <= f64::EPSILON {
        return base;
    }

    let noise_ratio =
        (NOISE_SIGMA_MULTIPLIER * (baseline_mad + current_mad) / baseline_median).clamp(0.0, 0.10);
    base + noise_ratio
}

fn regression_ratio(baseline: f64, current: f64, lower_is_worse: bool) -> f64 {
    if baseline == 0.0 {
        return 0.0;
    }
    if lower_is_worse {
        ((baseline - current) / baseline).max(0.0)
    } else {
        ((current - baseline) / baseline).max(0.0)
    }
}

fn pairwise_regression_probability(
    baseline_samples: &[f64],
    current_samples: &[f64],
    lower_is_worse: bool,
) -> f64 {
    if baseline_samples.is_empty() || current_samples.is_empty() {
        return 0.0;
    }

    let mut regressions = 0.0_f64;
    let mut comparisons = 0.0_f64;

    for baseline in baseline_samples {
        for current in current_samples {
            let is_regression = if lower_is_worse {
                current < baseline
            } else {
                current > baseline
            };

            comparisons += 1.0;
            if is_regression {
                regressions += 1.0;
            }
        }
    }

    if comparisons <= f64::EPSILON {
        return 0.0;
    }

    regressions / comparisons
}

fn runtime_scale_factor(baseline: &RuntimeCalibration, current: &RuntimeCalibration) -> f64 {
    if baseline.median_probe_ms <= f64::EPSILON || current.median_probe_ms <= f64::EPSILON {
        return 1.0;
    }
    let raw_scale = current.median_probe_ms / baseline.median_probe_ms;
    let baseline_low = (baseline.median_probe_ms - 3.0 * baseline.mad_probe_ms).max(f64::EPSILON);
    let baseline_high = (baseline.median_probe_ms + 3.0 * baseline.mad_probe_ms).max(baseline_low);
    let current_low = (current.median_probe_ms - 3.0 * current.mad_probe_ms).max(f64::EPSILON);
    let current_high = (current.median_probe_ms + 3.0 * current.mad_probe_ms).max(current_low);

    let min_scale = (current_low / baseline_high).max(f64::EPSILON);
    let max_scale = (current_high / baseline_low).max(min_scale);
    raw_scale.clamp(min_scale, max_scale)
}

fn scale_throughput(value: f64, runtime_scale: f64) -> f64 {
    value * runtime_scale
}

fn scale_latency(value: f64, runtime_scale: f64) -> f64 {
    value / runtime_scale
}

#[cfg(test)]
mod tests {
    use super::{
        check_first_page_latency_regression, check_throughput_regression,
        pairwise_regression_probability, regression_ratio, threshold_with_noise,
    };
    use crate::bench_cmd::validate_required_fixture_set;
    use crate::bench_cmd::{FixtureBenchResult, RuntimeCalibration};
    use std::collections::BTreeSet;

    #[test]
    fn regression_ratio_behaves_as_expected() {
        let throughput_drop = regression_ratio(100.0, 70.0, true);
        assert!((throughput_drop - 0.3).abs() < f64::EPSILON);

        let latency_increase = regression_ratio(100.0, 130.0, false);
        assert!((latency_increase - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn threshold_with_noise_is_tighter_than_legacy_defaults() {
        let threshold = threshold_with_noise(0.25, 40.0, 1.0, 1.0);
        assert!(threshold < 0.4);
    }

    #[test]
    fn fixture_set_validation_rejects_missing_fixture() {
        let actual = ["doclaynet_simple_text.pdf", "doclaynet_multi_column.pdf"]
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<String>>();

        assert!(validate_required_fixture_set(&actual).is_err());
    }

    #[test]
    fn pairwise_probability_detects_consistent_regression_direction() {
        let baseline = [10.0, 10.5, 11.0];
        let slower_current = [13.0, 13.5, 14.0];
        let probability = pairwise_regression_probability(&baseline, &slower_current, false);
        assert!(probability > 0.95);

        let faster_current = [8.5, 9.0, 9.5];
        let throughput_probability =
            pairwise_regression_probability(&baseline, &faster_current, true);
        assert!(throughput_probability > 0.95);
    }

    #[test]
    fn absolute_throughput_floor_is_enforced() {
        let baseline = sample_fixture("fixture", 20.0, 10.0);
        let current = sample_fixture("fixture", 20.0, 6.5);
        let result = check_throughput_regression(&baseline, &current, 1.0);
        assert!(result.is_err());
    }

    #[test]
    fn absolute_first_page_latency_cap_is_enforced() {
        let baseline = sample_fixture("fixture", 20.0, 10.0);
        let mut current = sample_fixture("fixture", 20.0, 10.0);
        current.median_first_page_ms = 40.0;
        current.first_page_ms_samples = vec![39.0, 40.0, 41.0];
        let result = check_first_page_latency_regression(&baseline, &current, 1.0);
        assert!(result.is_err());
    }

    #[test]
    fn runtime_scale_factor_uses_confidence_bounds() {
        let baseline = RuntimeCalibration {
            median_probe_ms: 10.0,
            mad_probe_ms: 1.0,
            probe_ms_samples: vec![9.0, 10.0, 11.0],
        };
        let current = RuntimeCalibration {
            median_probe_ms: 30.0,
            mad_probe_ms: 2.0,
            probe_ms_samples: vec![28.0, 30.0, 32.0],
        };

        let scale = super::runtime_scale_factor(&baseline, &current);
        assert!(scale > 1.5);
    }

    #[test]
    fn small_fixtures_skip_strict_latency_checks() {
        let mut baseline = sample_fixture("small", 5.0, 10.0);
        baseline.bytes = 512;
        let mut current = sample_fixture("small", 80.0, 10.0);
        current.bytes = 512;
        let result = super::check_fixture_regression(&baseline, &current, 1.0);
        assert!(result.is_ok());
    }

    fn sample_fixture(name: &str, first_page_ms: f64, throughput: f64) -> FixtureBenchResult {
        FixtureBenchResult {
            fixture: name.to_string(),
            bytes: 1000,
            warmup_iterations: 1,
            iterations: 3,
            median_full_ms: 20.0,
            mad_full_ms: 0.5,
            median_first_page_ms: first_page_ms,
            mad_first_page_ms: 0.5,
            median_throughput_mib_per_s: throughput,
            mad_throughput_mib_per_s: 0.5,
            full_ms_samples: vec![19.0, 20.0, 21.0],
            first_page_ms_samples: vec![first_page_ms - 1.0, first_page_ms, first_page_ms + 1.0],
            throughput_mib_per_s_samples: vec![throughput - 0.5, throughput, throughput + 0.5],
        }
    }
}
