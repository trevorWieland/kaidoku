use crate::{BenchCommand, BenchSubcommands, Phase1BenchCommand};
use anyhow::{Context, Result, bail};
use kaidoku_core::{ExtractOptions, extract_pdf, extract_pdf_first_page};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
mod baseline_store;
mod environment;
mod regression;
mod stats;
#[cfg(test)]
use baseline_store::validate_required_runner_classes;
use baseline_store::{load_bench_baseline_store, write_bench_report};
use environment::benchmark_environment;
use stats::{bytes_to_mib, mad, median, round_metric};

const BENCH_SCHEMA_VERSION: &str = "kaidoku.phase1.bench.v5";
const THROUGHPUT_MAX_REGRESSION_RATIO: f64 = 0.15;
const LATENCY_MAX_REGRESSION_RATIO: f64 = 0.15;
const NOISE_SIGMA_MULTIPLIER: f64 = 2.0;
const REGRESSION_PROBABILITY_THRESHOLD: f64 = 0.65;
const REGRESSION_EFFECT_SIZE_FLOOR: f64 = 0.05;
const FULL_LATENCY_ABSOLUTE_MS_SLACK: f64 = 8.0;
const FIRST_PAGE_LATENCY_ABSOLUTE_MS_SLACK: f64 = 6.0;
const THROUGHPUT_ABSOLUTE_MIB_DELTA: f64 = 1.0;
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;
const REQUIRED_PHASE1_FIXTURES: [&str; 6] = [
    "doclaynet_simple_text.pdf",
    "doclaynet_multi_column.pdf",
    "doclaynet_mixed_content.pdf",
    "pdfjs_copy_paste_ligatures.pdf",
    "pdfjs_arabic_cid_true_type.pdf",
    "pdfjs_identity_to_unicode_map_char_code_of.pdf",
];

/// Runner classes that MUST have a baseline entry in
/// `tests/golden/phase1/benchmarks.baseline.json` at all times.
///
/// If any class is missing the bench gate fails fast with a precise error
/// instead of the CI job quietly succeeding on only some runners. Seeding new
/// entries is a deliberate operator action via `just phase1-bench-refresh` on
/// the corresponding runner.
pub(super) const REQUIRED_BASELINE_RUNNER_CLASSES: &[&str] = &[
    // Local dev machine (M1 Pro / M3 Pro tier with rustc 1.94).
    "macos-aarch64-release-core-large-rustc1.94",
    // GitHub Actions macOS runner (aarch64, small core tier, pinned rustc 1.85).
    "macos-aarch64-release-core-small-rustc1.85",
    // GitHub Actions Ubuntu runner (x86_64, small core tier, pinned rustc 1.85).
    "linux-x86_64-release-core-small-rustc1.85",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BenchReport {
    schema_version: String,
    environment: BenchEnvironment,
    calibration: RuntimeCalibration,
    fixtures: Vec<FixtureBenchResult>,
    checks: BenchChecks,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct BenchEnvironment {
    generated_at_unix_seconds: u64,
    /// Fine-grained CI-gate class: OS + arch + profile + CPU-tier + rustc-minor.
    runner_class: String,
    /// Coarse compatibility class: OS + arch + profile only.
    ///
    /// Defaulted to empty on `v4` baselines; the store loader backfills it on
    /// first read so historical `v4` files deserialize cleanly.
    #[serde(default)]
    compat_class: String,
    /// Which rule selected the baseline used for comparison. Recorded in the
    /// CURRENT report only (never in the baseline store itself) so CI logs
    /// show whether a strict or compat match was used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    class_match: Option<String>,
    os: String,
    arch: String,
    cpu_logical_cores: usize,
    profile: String,
    rustc_version: String,
    hostname: Option<String>,
    cpu_governor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BenchBaselineStore {
    schema_version: String,
    reports: Vec<BenchReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeCalibration {
    median_probe_ms: f64,
    mad_probe_ms: f64,
    probe_ms_samples: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BenchChecks {
    warmup_iterations: u32,
    throughput_max_regression_ratio: f64,
    latency_max_regression_ratio: f64,
    throughput_absolute_mib_delta: f64,
    full_latency_absolute_ms_slack: f64,
    first_page_latency_absolute_ms_slack: f64,
    noise_sigma_multiplier: f64,
    regression_probability_threshold: f64,
    regression_effect_size_floor: f64,
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
    full_ms_samples: Vec<f64>,
    first_page_ms_samples: Vec<f64>,
    throughput_mib_per_s_samples: Vec<f64>,
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
    let calibration = benchmark_runtime_calibration(command.iterations, command.warmup_iterations)?;

    let report = BenchReport {
        schema_version: BENCH_SCHEMA_VERSION.to_string(),
        environment: benchmark_environment(),
        calibration,
        fixtures: results,
        checks: BenchChecks {
            warmup_iterations: command.warmup_iterations,
            throughput_max_regression_ratio: THROUGHPUT_MAX_REGRESSION_RATIO,
            latency_max_regression_ratio: LATENCY_MAX_REGRESSION_RATIO,
            throughput_absolute_mib_delta: THROUGHPUT_ABSOLUTE_MIB_DELTA,
            full_latency_absolute_ms_slack: FULL_LATENCY_ABSOLUTE_MS_SLACK,
            first_page_latency_absolute_ms_slack: FIRST_PAGE_LATENCY_ABSOLUTE_MS_SLACK,
            noise_sigma_multiplier: NOISE_SIGMA_MULTIPLIER,
            regression_probability_threshold: REGRESSION_PROBABILITY_THRESHOLD,
            regression_effect_size_floor: REGRESSION_EFFECT_SIZE_FLOOR,
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

    write_bench_report(&command.output, &report)?;

    if command.check {
        let strict = bench_strict_mode(command.strict);
        let (baseline, match_kind) = load_baseline_with_fallback(
            &command.baseline,
            &report.environment.runner_class,
            &report.environment.compat_class,
            strict,
        )?;
        regression::check_bench_regression(&baseline, &report)?;

        // Persist the resolved match kind into the current report so CI logs
        // and human review can see whether strict or compat matching landed.
        let mut report_with_match = report;
        report_with_match.environment.class_match = Some(match_kind);
        write_bench_report(&command.output, &report_with_match)?;
    }

    Ok(())
}

fn bench_strict_mode(cli_flag: bool) -> bool {
    if cli_flag {
        return true;
    }
    std::env::var("KAIDOKU_BENCH_STRICT")
        .ok()
        .is_some_and(|value| !value.is_empty() && value != "0")
}

fn load_baseline_with_fallback(
    path: &Path,
    runner_class: &str,
    compat_class: &str,
    strict: bool,
) -> Result<(BenchReport, String)> {
    let store = load_bench_baseline_store(path)?;

    if let Some(report) = store
        .reports
        .iter()
        .find(|candidate| candidate.environment.runner_class == runner_class)
    {
        return Ok((report.clone(), format!("strict:{runner_class}")));
    }

    if strict {
        let available = store
            .reports
            .iter()
            .map(|report| report.environment.runner_class.clone())
            .collect::<Vec<_>>();
        bail!(
            "benchmark baseline {} is missing runner class `{}` and strict mode is on; \
             available classes: [{}]. Seed this class with `just phase1-bench-refresh`.",
            path.display(),
            runner_class,
            available.join(", "),
        );
    }

    if let Some(report) = store.reports.iter().find(|candidate| {
        !candidate.environment.compat_class.is_empty()
            && candidate.environment.compat_class == compat_class
    }) {
        return Ok((report.clone(), format!("compat:{compat_class}")));
    }

    bail!(
        "benchmark baseline {} has no entry matching runner_class `{}` or compat_class `{}`; \
         seed one via `just phase1-bench-refresh`.",
        path.display(),
        runner_class,
        compat_class,
    )
}

fn benchmark_runtime_calibration(
    iterations: u32,
    warmup_iterations: u32,
) -> Result<RuntimeCalibration> {
    for _ in 0..warmup_iterations {
        run_calibration_probe();
    }

    let mut probe_runs = Vec::with_capacity(
        usize::try_from(iterations).context("iteration count does not fit into usize")?,
    );
    for _ in 0..iterations {
        let start = Instant::now();
        run_calibration_probe();
        probe_runs.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    let median_probe_ms = median(&mut probe_runs);
    Ok(RuntimeCalibration {
        median_probe_ms: round_metric(median_probe_ms),
        mad_probe_ms: round_metric(mad(&probe_runs, median_probe_ms)),
        probe_ms_samples: probe_runs.iter().copied().map(round_metric).collect(),
    })
}

fn run_calibration_probe() {
    let mut state = 0_u64;
    for i in 0_u64..500_000 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(i ^ 0x9E37_79B9_7F4A_7C15);
    }
    std::hint::black_box(state);
}

fn benchmark_fixture(
    path: &Path,
    iterations: u32,
    warmup_iterations: u32,
) -> Result<FixtureBenchResult> {
    let bytes =
        fs::read(path).with_context(|| format!("failed reading fixture {}", path.display()))?;
    let bytes_len = bytes.len();
    let bytes_u64 = u64::try_from(bytes_len).context("fixture size does not fit into u64")?;
    let fixture_mib = bytes_to_mib(bytes_u64);

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

        let full_seconds = elapsed_full_ms / 1000.0;
        let throughput_mib_per_s = if full_seconds == 0.0 {
            0.0
        } else {
            fixture_mib / full_seconds
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
        full_ms_samples: full_runs.iter().copied().map(round_metric).collect(),
        first_page_ms_samples: first_page_runs.iter().copied().map(round_metric).collect(),
        throughput_mib_per_s_samples: throughput_runs.iter().copied().map(round_metric).collect(),
    })
}

fn run_full_extraction(bytes: &[u8], path: &Path) -> Result<()> {
    let _document = extract_pdf(bytes, ExtractOptions::default())
        .with_context(|| format!("full extraction failed for {}", path.display()))?;
    Ok(())
}

fn run_first_page_extraction(bytes: &[u8], path: &Path) -> Result<()> {
    // Use the dedicated first-page fast path: this avoids the full page-tree
    // walk and only extracts the first page, yielding a realistic
    // latency-to-first-page metric for large PDFs.
    let _first_page_doc = extract_pdf_first_page(bytes, ExtractOptions::default())
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

#[cfg(test)]
mod tests;
