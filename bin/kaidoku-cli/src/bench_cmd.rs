use crate::{BenchCommand, BenchSubcommands, Phase1BenchCommand};
use anyhow::{Context, Result, bail};
use kaidoku_core::{ExtractOptions, PageRange, PageSelection, extract_pdf};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

mod regression;

const BENCH_SCHEMA_VERSION: &str = "kaidoku.phase1.bench.v3";
const THROUGHPUT_MAX_REGRESSION_RATIO: f64 = 0.15;
const LATENCY_MAX_REGRESSION_RATIO: f64 = 0.15;
const NOISE_SIGMA_MULTIPLIER: f64 = 2.0;
const REGRESSION_PROBABILITY_THRESHOLD: f64 = 0.65;
const REGRESSION_EFFECT_SIZE_FLOOR: f64 = 0.05;
const FULL_LATENCY_ABSOLUTE_MS_SLACK: f64 = 8.0;
const FIRST_PAGE_LATENCY_ABSOLUTE_MS_SLACK: f64 = 6.0;
const THROUGHPUT_ABSOLUTE_MIB_DELTA: f64 = 1.0;
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;
const REQUIRED_PHASE1_FIXTURES: [&str; 3] = [
    "doclaynet_simple_text.pdf",
    "doclaynet_multi_column.pdf",
    "doclaynet_mixed_content.pdf",
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
    os: String,
    arch: String,
    cpu_logical_cores: usize,
    profile: String,
    rustc_version: String,
    hostname: Option<String>,
    cpu_governor: Option<String>,
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
        regression::check_bench_regression(&baseline, &report)?;
    }

    Ok(())
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

fn benchmark_environment() -> BenchEnvironment {
    BenchEnvironment {
        generated_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map_or(0, |duration| duration.as_secs()),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu_logical_cores: thread::available_parallelism().map_or(1, usize::from),
        profile: if cfg!(debug_assertions) {
            "debug".to_string()
        } else {
            "release".to_string()
        },
        rustc_version: rustc_version(),
        hostname: hostname(),
        cpu_governor: cpu_governor(),
    }
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map_or_else(|| "unknown".to_string(), |value| value.trim().to_string())
}

fn hostname() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
}

fn cpu_governor() -> Option<String> {
    if std::env::consts::OS != "linux" {
        return None;
    }

    fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
        .ok()
        .map(|value| value.trim().to_string())
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
    let bytes_u32 = u32::try_from(bytes_len)
        .context("fixture size does not fit into u32 for throughput math")?;
    let fixture_mib = f64::from(bytes_u32) / BYTES_PER_MIB;

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
    let first_page_options = ExtractOptions::builder()
        .page_selection(PageSelection::Range(PageRange::new(1, 1)?))
        .build()?;
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
