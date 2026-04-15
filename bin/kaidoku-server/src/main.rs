use anyhow::Result;
use axum::{Router, routing::get};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "kaidoku-server", about = "Kaidoku server scaffold")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let app = Router::new().route("/healthz", get(health));
    let listener = tokio::net::TcpListener::bind(&cli.bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}
