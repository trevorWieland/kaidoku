use crate::{BenchCommand, BenchSubcommands, Phase1BenchCommand};
use anyhow::{Context, Result, bail};
use kaidoku_core::{ExtractOptions, PageRange, PageSelection, extract_pdf};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

const BENCH_SCHEMA_VERSION: &str = "kaidoku.phase1.bench.v2";
const THROUGHPUT_MAX_REGRESSION_RATIO: f64 = 0.20;
const LATENCY_MAX_REGRESSION_RATIO: f64 = 0.20;
const NOISE_SIGMA_MULTIPLIER: f64 = 2.5;
const REQUIRED_PHASE1_FIXTURES: [&str; 3] = [
    "doclaynet_simple_text.pdf",
    "doclaynet_multi_column.pdf",
    "doclaynet_mixed_content.pdf",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BenchReport {
    schema_version: String,
    fixtures: Vec<FixtureBenchResult>,
    checks: BenchChecks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BenchChecks {
    warmup_iterations: u32,
    throughput_max_regression_ratio: f64,
    latency_max_regression_ratio: f64,
    noise_sigma_multiplier: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FixtureBenchResult {
    fixture: String,
    bytes: u64,
    warmup_iterations: u32,
    iterations: u32,
    median_full_ms: f64,
    mad_full_ms: f64,
    median_first_page_ms: f64,
    mad_first_page_ms: f64,
    median_throughput_mib_per_s: f64,
    mad_throughput_mib_per_s: f64,
}

pub(super) fn run_bench(command: BenchCommand) -> Result<()> {
    match command.subcommand {
        BenchSubcommands::Phase1(phase1) => run_phase1_bench(&phase1),
    }
}

fn run_phase1_bench(command: &Phase1BenchCommand) -> Result<()> {
    if command.iterations == 0 {
        bail!("--iterations must be >= 1");
    }

    let fixtures = collect_phase1_fixture_paths(&command.fixtures)?;

    let mut results = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        results.push(benchmark_fixture(
            &fixture,
            command.iterations,
            command.warmup_iterations,
        )?);
    }

    let report = BenchReport {
        schema_version: BENCH_SCHEMA_VERSION.to_string(),
        fixtures: results,
        checks: BenchChecks {
            warmup_iterations: command.warmup_iterations,
            throughput_max_regression_ratio: THROUGHPUT_MAX_REGRESSION_RATIO,
            latency_max_regression_ratio: LATENCY_MAX_REGRESSION_RATIO,
            noise_sigma_multiplier: NOISE_SIGMA_MULTIPLIER,
        },
    };

    fs::create_dir_all(command.output.parent().unwrap_or_else(|| Path::new("."))).with_context(
        || {
            format!(
                "failed creating benchmark output directory for {}",
                command.output.display()
            )
        },
    )?;

    let mut serialized = serde_json::to_string_pretty(&report)?;
    serialized.push('\n');
    fs::write(&command.output, serialized).with_context(|| {
        format!(
            "failed writing benchmark report {}",
            command.output.display()
        )
    })?;

    if command.check {
        let baseline = load_bench_report(&command.baseline)?;
        check_bench_regression(&baseline, &report)?;
    }

    Ok(())
}

fn benchmark_fixture(
    path: &Path,
    iterations: u32,
    warmup_iterations: u32,
) -> Result<FixtureBenchResult> {
    let bytes =
        fs::read(path).with_context(|| format!("failed reading fixture {}", path.display()))?;
    let bytes_u64 = u64::try_from(bytes.len()).context("fixture size does not fit into u64")?;

    for _ in 0..warmup_iterations {
        run_full_extraction(&bytes, path)?;
        run_first_page_extraction(&bytes, path)?;
    }

    let mut full_runs = Vec::with_capacity(
        usize::try_from(iterations).context("iteration count does not fit into usize")?,
    );
    let mut first_page_runs = Vec::with_capacity(
        usize::try_from(iterations).context("iteration count does not fit into usize")?,
    );
    let mut throughput_runs = Vec::with_capacity(
        usize::try_from(iterations).context("iteration count does not fit into usize")?,
    );

    for _ in 0..iterations {
        let start = Instant::now();
        run_full_extraction(&bytes, path)?;
        let elapsed_full_ms = start.elapsed().as_secs_f64() * 1000.0;
        full_runs.push(elapsed_full_ms);

        let bytes_as_f64: f64 = bytes_u64
            .to_string()
            .parse()
            .context("failed converting bytes to f64")?;
        let full_seconds = elapsed_full_ms / 1000.0;
        let throughput_mib_per_s = if full_seconds == 0.0 {
            0.0
        } else {
            (bytes_as_f64 / (1024.0 * 1024.0)) / full_seconds
        };
        throughput_runs.push(throughput_mib_per_s);

        let start = Instant::now();
        run_first_page_extraction(&bytes, path)?;
        first_page_runs.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    let median_full_ms = median(&mut full_runs);
    let median_first_page_ms = median(&mut first_page_runs);
    let median_throughput_mib_per_s = median(&mut throughput_runs);

    Ok(FixtureBenchResult {
        fixture: path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| anyhow::anyhow!("invalid fixture filename: {}", path.display()))?
            .to_string(),
        bytes: bytes_u64,
        warmup_iterations,
        iterations,
        median_full_ms: round_metric(median_full_ms),
        mad_full_ms: round_metric(mad(&full_runs, median_full_ms)),
        median_first_page_ms: round_metric(median_first_page_ms),
        mad_first_page_ms: round_metric(mad(&first_page_runs, median_first_page_ms)),
        median_throughput_mib_per_s: round_metric(median_throughput_mib_per_s),
        mad_throughput_mib_per_s: round_metric(mad(&throughput_runs, median_throughput_mib_per_s)),
    })
}

fn run_full_extraction(bytes: &[u8], path: &Path) -> Result<()> {
    let _document = extract_pdf(bytes, ExtractOptions::default())
        .with_context(|| format!("full extraction failed for {}", path.display()))?;
    Ok(())
}

fn run_first_page_extraction(bytes: &[u8], path: &Path) -> Result<()> {
    let first_page_options = ExtractOptions {
        page_selection: PageSelection::Range(PageRange::new(1, 1)?),
        ..ExtractOptions::default()
    };
    let _first_page_doc = extract_pdf(bytes, first_page_options)
        .with_context(|| format!("first-page extraction failed for {}", path.display()))?;
    Ok(())
}

fn collect_phase1_fixture_paths(fixtures_dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(fixtures_dir).with_context(|| {
        format!(
            "failed listing fixtures directory {}",
            fixtures_dir.display()
        )
    })?;

    let mut by_name = BTreeMap::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension() == Some(OsStr::new("pdf"))
            && let Some(name) = path.file_name().and_then(OsStr::to_str)
        {
            by_name.insert(name.to_string(), path);
        }
    }

    let actual = by_name.keys().cloned().collect::<BTreeSet<String>>();
    validate_required_fixture_set(&actual)?;

    REQUIRED_PHASE1_FIXTURES
        .iter()
        .map(|name| {
            by_name
                .get(*name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("required fixture missing after validation: {name}"))
        })
        .collect()
}

fn validate_required_fixture_set(actual: &BTreeSet<String>) -> Result<()> {
    let expected = REQUIRED_PHASE1_FIXTURES
        .iter()
        .map(|name| (*name).to_string())
        .collect::<BTreeSet<String>>();

    if *actual == expected {
        return Ok(());
    }

    let missing = expected
        .difference(actual)
        .cloned()
        .collect::<Vec<String>>();
    let extra = actual
        .difference(&expected)
        .cloned()
        .collect::<Vec<String>>();

    bail!(
        "phase1 benchmark fixture set mismatch; missing: [{}], extra: [{}]",
        missing.join(", "),
        extra.join(", ")
    );
}

fn load_bench_report(path: &Path) -> Result<BenchReport> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed reading baseline benchmark {}", path.display()))?;

    let report: BenchReport = serde_json::from_str(&content)
        .with_context(|| format!("failed parsing baseline benchmark {}", path.display()))?;

    if report.schema_version != BENCH_SCHEMA_VERSION {
        bail!(
            "unexpected benchmark schema {} in {}",
            report.schema_version,
            path.display(),
        );
    }

    Ok(report)
}

fn check_bench_regression(baseline: &BenchReport, current: &BenchReport) -> Result<()> {
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

        let throughput_drop = regression_ratio(
            previous.median_throughput_mib_per_s,
            fixture.median_throughput_mib_per_s,
            true,
        );
        let throughput_limit = threshold_with_noise(
            THROUGHPUT_MAX_REGRESSION_RATIO,
            previous.median_throughput_mib_per_s,
            previous.mad_throughput_mib_per_s,
            fixture.mad_throughput_mib_per_s,
        );
        if throughput_drop > throughput_limit {
            bail!(
                "throughput regression for {} is {:.3}, exceeds {:.3}",
                fixture.fixture,
                throughput_drop,
                throughput_limit,
            );
        }

        let full_latency_increase =
            regression_ratio(previous.median_full_ms, fixture.median_full_ms, false);
        let full_limit = threshold_with_noise(
            LATENCY_MAX_REGRESSION_RATIO,
            previous.median_full_ms,
            previous.mad_full_ms,
            fixture.mad_full_ms,
        );
        if full_latency_increase > full_limit {
            bail!(
                "full extraction latency regression for {} is {:.3}, exceeds {:.3}",
                fixture.fixture,
                full_latency_increase,
                full_limit,
            );
        }

        let first_page_latency_increase = regression_ratio(
            previous.median_first_page_ms,
            fixture.median_first_page_ms,
            false,
        );
        let first_page_limit = threshold_with_noise(
            LATENCY_MAX_REGRESSION_RATIO,
            previous.median_first_page_ms,
            previous.mad_first_page_ms,
            fixture.mad_first_page_ms,
        );
        if first_page_latency_increase > first_page_limit {
            bail!(
                "first-page latency regression for {} is {:.3}, exceeds {:.3}",
                fixture.fixture,
                first_page_latency_increase,
                first_page_limit,
            );
        }
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

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let midpoint = values.len() / 2;

    if values.len() % 2 == 0 {
        f64::midpoint(values[midpoint - 1], values[midpoint])
    } else {
        values[midpoint]
    }
}

fn mad(values: &[f64], median_value: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let mut deviations = values
        .iter()
        .map(|value| (value - median_value).abs())
        .collect::<Vec<f64>>();
    median(&mut deviations)
}

fn round_metric(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::{BTreeSet, regression_ratio, threshold_with_noise, validate_required_fixture_set};

    #[test]
    fn regression_ratio_behaves_as_expected() {
        let throughput_drop = regression_ratio(100.0, 70.0, true);
        assert!((throughput_drop - 0.3).abs() < f64::EPSILON);

        let latency_increase = regression_ratio(100.0, 130.0, false);
        assert!((latency_increase - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn threshold_with_noise_is_tighter_than_legacy_defaults() {
        let threshold = threshold_with_noise(0.2, 40.0, 1.0, 1.0);
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
}
