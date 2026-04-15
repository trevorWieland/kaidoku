use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use kaidoku_core::{
    ExtractOptions, ExtractionDocument, PageRange, PageSelection, extract_pdf, to_canonical_json,
};
use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

const BENCH_SCHEMA_VERSION: &str = "kaidoku.phase1.bench.v1";
const THROUGHPUT_MAX_REGRESSION_RATIO: f64 = 0.40;
const LATENCY_MAX_REGRESSION_RATIO: f64 = 0.40;

#[derive(Debug, Parser)]
#[command(name = "kaidoku", about = "Kaidoku CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Extract(ExtractCommand),
    Bench(BenchCommand),
}

#[derive(Debug, Args)]
struct ExtractCommand {
    #[arg(long, required = true)]
    input: Vec<PathBuf>,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    pages: Option<String>,
    #[arg(long, default_value_t = 1)]
    jobs: usize,
}

#[derive(Debug, Args)]
struct BenchCommand {
    #[command(subcommand)]
    subcommand: BenchSubcommands,
}

#[derive(Debug, Subcommand)]
enum BenchSubcommands {
    Phase1(Phase1BenchCommand),
}

#[derive(Debug, Args)]
struct Phase1BenchCommand {
    #[arg(long, default_value_t = 7)]
    iterations: u32,
    #[arg(long, default_value = "tests/corpus/phase1")]
    fixtures: PathBuf,
    #[arg(long, default_value = "tests/golden/phase1/benchmarks.current.json")]
    output: PathBuf,
    #[arg(long, default_value = "tests/golden/phase1/benchmarks.baseline.json")]
    baseline: PathBuf,
    #[arg(long, default_value_t = false)]
    check: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BenchReport {
    schema_version: String,
    fixtures: Vec<FixtureBenchResult>,
    checks: BenchChecks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BenchChecks {
    throughput_max_regression_ratio: f64,
    latency_max_regression_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FixtureBenchResult {
    fixture: String,
    bytes: u64,
    iterations: u32,
    median_full_ms: f64,
    median_first_page_ms: f64,
    median_throughput_mib_per_s: f64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Extract(command) => run_extract(command),
        Commands::Bench(command) => run_bench(command),
    }
}

fn run_extract(command: ExtractCommand) -> Result<()> {
    if command.jobs == 0 {
        bail!("--jobs must be >= 1");
    }

    let selection = parse_pages_spec(command.pages.as_deref())?;
    fs::create_dir_all(&command.output).with_context(|| {
        format!(
            "failed creating output directory {}",
            command.output.display()
        )
    })?;

    let mut inputs = command.input;
    inputs.sort();

    let pool = ThreadPoolBuilder::new()
        .num_threads(command.jobs)
        .build()
        .context("failed to build extraction thread pool")?;

    pool.install(|| {
        inputs
            .par_iter()
            .map(|input_path| extract_one(input_path, &command.output, selection.clone()))
            .collect::<Result<Vec<_>>>()
    })?;

    Ok(())
}

fn extract_one(input_path: &Path, output_dir: &Path, page_selection: PageSelection) -> Result<()> {
    let bytes = fs::read(input_path)
        .with_context(|| format!("failed reading input file {}", input_path.display()))?;

    let options = ExtractOptions {
        page_selection,
        ..ExtractOptions::default()
    };

    let document = extract_pdf(&bytes, options)
        .with_context(|| format!("failed extracting {}", input_path.display()))?;

    let json = to_canonical_json(&document)?;
    let output_path = extraction_output_path(input_path, output_dir, &document)?;
    fs::write(&output_path, json)
        .with_context(|| format!("failed writing output {}", output_path.display()))?;

    Ok(())
}

fn extraction_output_path(
    input_path: &Path,
    output_dir: &Path,
    _document: &ExtractionDocument,
) -> Result<PathBuf> {
    let stem = input_path
        .file_stem()
        .and_then(OsStr::to_str)
        .ok_or_else(|| anyhow!("invalid input filename: {}", input_path.display()))?;

    Ok(output_dir.join(format!("{stem}.json")))
}

fn run_bench(command: BenchCommand) -> Result<()> {
    match command.subcommand {
        BenchSubcommands::Phase1(phase1) => run_phase1_bench(&phase1),
    }
}

fn run_phase1_bench(command: &Phase1BenchCommand) -> Result<()> {
    if command.iterations == 0 {
        bail!("--iterations must be >= 1");
    }

    let fixtures = collect_fixture_paths(&command.fixtures)?;
    if fixtures.is_empty() {
        bail!("no PDF fixtures found in {}", command.fixtures.display());
    }

    let mut results = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        results.push(benchmark_fixture(&fixture, command.iterations)?);
    }

    let report = BenchReport {
        schema_version: BENCH_SCHEMA_VERSION.to_string(),
        fixtures: results,
        checks: BenchChecks {
            throughput_max_regression_ratio: THROUGHPUT_MAX_REGRESSION_RATIO,
            latency_max_regression_ratio: LATENCY_MAX_REGRESSION_RATIO,
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

fn benchmark_fixture(path: &Path, iterations: u32) -> Result<FixtureBenchResult> {
    let bytes =
        fs::read(path).with_context(|| format!("failed reading fixture {}", path.display()))?;
    let bytes_u64 = u64::try_from(bytes.len()).context("fixture size does not fit into u64")?;

    let mut full_runs = Vec::with_capacity(
        usize::try_from(iterations).context("iteration count does not fit into usize")?,
    );
    let mut first_page_runs = Vec::with_capacity(
        usize::try_from(iterations).context("iteration count does not fit into usize")?,
    );

    for _ in 0..iterations {
        let start = Instant::now();
        let _document = extract_pdf(&bytes, ExtractOptions::default())
            .with_context(|| format!("full extraction failed for {}", path.display()))?;
        full_runs.push(start.elapsed().as_secs_f64() * 1000.0);

        let first_page_options = ExtractOptions {
            page_selection: PageSelection::Range(PageRange::new(1, 1)?),
            ..ExtractOptions::default()
        };
        let start = Instant::now();
        let _first_page_doc = extract_pdf(&bytes, first_page_options)
            .with_context(|| format!("first-page extraction failed for {}", path.display()))?;
        first_page_runs.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    let median_full_ms = median(&mut full_runs);
    let median_first_page_ms = median(&mut first_page_runs);

    let bytes_as_f64: f64 = bytes_u64
        .to_string()
        .parse()
        .context("failed converting bytes to f64")?;
    let full_seconds = median_full_ms / 1000.0;
    let throughput_mib_per_s = if full_seconds == 0.0 {
        0.0
    } else {
        (bytes_as_f64 / (1024.0 * 1024.0)) / full_seconds
    };

    Ok(FixtureBenchResult {
        fixture: path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| anyhow!("invalid fixture filename: {}", path.display()))?
            .to_string(),
        bytes: bytes_u64,
        iterations,
        median_full_ms: round_metric(median_full_ms),
        median_first_page_ms: round_metric(median_first_page_ms),
        median_throughput_mib_per_s: round_metric(throughput_mib_per_s),
    })
}

fn collect_fixture_paths(fixtures_dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir(fixtures_dir).with_context(|| {
        format!(
            "failed listing fixtures directory {}",
            fixtures_dir.display()
        )
    })?;

    let mut fixtures = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension() == Some(OsStr::new("pdf")) {
            fixtures.push(path);
        }
    }

    fixtures.sort();
    Ok(fixtures)
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
        if throughput_drop > THROUGHPUT_MAX_REGRESSION_RATIO {
            bail!(
                "throughput regression for {} is {:.3}, exceeds {:.3}",
                fixture.fixture,
                throughput_drop,
                THROUGHPUT_MAX_REGRESSION_RATIO,
            );
        }

        let full_latency_increase =
            regression_ratio(previous.median_full_ms, fixture.median_full_ms, false);
        if full_latency_increase > LATENCY_MAX_REGRESSION_RATIO {
            bail!(
                "full extraction latency regression for {} is {:.3}, exceeds {:.3}",
                fixture.fixture,
                full_latency_increase,
                LATENCY_MAX_REGRESSION_RATIO,
            );
        }

        let first_page_latency_increase = regression_ratio(
            previous.median_first_page_ms,
            fixture.median_first_page_ms,
            false,
        );
        if first_page_latency_increase > LATENCY_MAX_REGRESSION_RATIO {
            bail!(
                "first-page latency regression for {} is {:.3}, exceeds {:.3}",
                fixture.fixture,
                first_page_latency_increase,
                LATENCY_MAX_REGRESSION_RATIO,
            );
        }
    }

    Ok(())
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

fn parse_pages_spec(spec: Option<&str>) -> Result<PageSelection> {
    let Some(raw_spec) = spec else {
        return Ok(PageSelection::All);
    };

    let trimmed = raw_spec.trim();
    if trimmed.eq_ignore_ascii_case("all") {
        return Ok(PageSelection::All);
    }

    let mut explicit_pages = Vec::new();
    let mut parsed_range: Option<PageRange> = None;

    for token in trimmed.split(',') {
        let entry = token.trim();
        if entry.is_empty() {
            continue;
        }

        if let Some((start, end)) = entry.split_once('-') {
            let start_page: u32 = start
                .trim()
                .parse()
                .with_context(|| format!("invalid page in range: {entry}"))?;
            let end_page: u32 = end
                .trim()
                .parse()
                .with_context(|| format!("invalid page in range: {entry}"))?;
            let range = PageRange::new(start_page, end_page)?;
            if trimmed.split(',').count() == 1 {
                parsed_range = Some(range);
            } else {
                for page in range.start..=range.end {
                    explicit_pages.push(page);
                }
            }
        } else {
            let page: u32 = entry
                .parse()
                .with_context(|| format!("invalid page entry: {entry}"))?;
            explicit_pages.push(page);
        }
    }

    if let Some(range) = parsed_range {
        return Ok(PageSelection::Range(range));
    }

    if explicit_pages.is_empty() {
        bail!("--pages produced an empty selection");
    }

    Ok(PageSelection::from_pages(explicit_pages)?)
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

fn round_metric(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::{PageSelection, parse_pages_spec, regression_ratio};

    #[test]
    fn pages_spec_all_defaults() {
        let selection = parse_pages_spec(None);
        assert!(selection.is_ok());

        let Ok(selection) = selection else { return };
        assert!(matches!(selection, PageSelection::All));
    }

    #[test]
    fn pages_spec_range_parses() {
        let selection = parse_pages_spec(Some("2-5"));
        assert!(selection.is_ok());

        let Ok(selection) = selection else { return };
        assert!(matches!(selection, PageSelection::Range(_)));
    }

    #[test]
    fn pages_spec_list_parses_to_explicit() {
        let selection = parse_pages_spec(Some("3,1,3,2"));
        assert!(selection.is_ok());

        let Ok(selection) = selection else { return };
        assert!(matches!(selection, PageSelection::Explicit(_)));
    }

    #[test]
    fn regression_ratio_behaves_as_expected() {
        let throughput_drop = regression_ratio(100.0, 70.0, true);
        assert!((throughput_drop - 0.3).abs() < f64::EPSILON);

        let latency_increase = regression_ratio(100.0, 130.0, false);
        assert!((latency_increase - 0.3).abs() < f64::EPSILON);
    }
}
