mod bench_cmd;
mod extract_cmd;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

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
    #[arg(long, default_value_t = kaidoku_core::default_max_wall_time_ms())]
    max_wall_time_ms: u64,
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
    #[arg(long, default_value_t = 15)]
    iterations: u32,
    #[arg(long, default_value_t = 4)]
    warmup_iterations: u32,
    #[arg(long, default_value = "tests/corpus/phase1")]
    fixtures: PathBuf,
    #[arg(long, default_value = "tests/golden/phase1/benchmarks.current.json")]
    output: PathBuf,
    #[arg(long, default_value = "tests/golden/phase1/benchmarks.baseline.json")]
    baseline: PathBuf,
    #[arg(long, default_value_t = false)]
    check: bool,
    /// Require exact `runner_class` match. Without this flag (and without
    /// `KAIDOKU_BENCH_STRICT=1`), the gate falls back to the coarser
    /// `compat_class` (os-arch-profile) when the strict class is absent.
    #[arg(long, default_value_t = false)]
    strict: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Extract(command) => extract_cmd::run_extract(command),
        Commands::Bench(command) => bench_cmd::run_bench(command),
    }
}
