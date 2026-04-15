use anyhow::{Result, bail};
use clap::Parser;
use kaidoku_core::PageRange;

#[derive(Debug, Parser)]
#[command(name = "kaidoku", about = "Kaidoku CLI scaffold")]
struct Cli {
    #[arg(long, default_value_t = 1)]
    start_page: u32,
    #[arg(long, default_value_t = 1)]
    end_page: u32,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let range = PageRange::new(cli.start_page, cli.end_page)?;

    if range.is_empty() {
        bail!("page range length must be positive");
    }

    Ok(())
}
