mod backtest;
mod collector;
mod domain;
mod forecast;
mod forecast_batch;
mod service;
mod slack;
mod store;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use reqwest::Url;
use tracing_subscriber::EnvFilter;

use crate::collector::Collector;
use crate::service::Service;
use crate::slack::Slack;
use crate::store::Store;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    #[arg(
        long,
        global = true,
        env = "LOTTERY_DB",
        default_value = "var/lottery.sqlite3"
    )]
    db: PathBuf,
    #[arg(
        long,
        global = true,
        env = "LOTTERY_SEED",
        default_value = "lottery.sqlite3"
    )]
    seed: PathBuf,
    #[arg(
        long,
        global = true,
        env = "LOTTERY_BASE_URL",
        default_value = "https://www.dhlottery.co.kr/"
    )]
    base_url: Url,
    #[arg(long, global = true, env = "LOTTERY_SLACK_WEBHOOK_FILE")]
    slack_webhook_file: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Poll for announced results until interrupted")]
    Run {
        #[arg(
            long,
            env = "LOTTERY_INTERVAL_SECONDS",
            default_value_t = 900,
            value_parser = clap::value_parser!(u64).range(1..=86_400)
        )]
        interval_seconds: u64,
    },
    #[command(about = "Collect and generate once; exit nonzero if either feed fails")]
    Once,
    #[command(about = "Read the latest stored draw and current candidate for each game as JSON")]
    Status,
    #[command(about = "Backtest sequential predictions against a read-only historical database")]
    Backtest,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(io::stderr)
        .json()
        .init();
    match args.command {
        Command::Status => {
            serde_json::to_writer_pretty(io::stdout().lock(), &Store::status(&args.db)?)?;
            return Ok(());
        }
        Command::Backtest => {
            serde_json::to_writer_pretty(io::stdout().lock(), &backtest::run(&args.db)?)?;
            return Ok(());
        }
        Command::Run { .. } | Command::Once => {}
    }
    let collector = Collector::new(args.base_url)?;
    let slack = args
        .slack_webhook_file
        .as_deref()
        .map(Slack::new)
        .transpose()?;
    let store = tokio::task::spawn_blocking(move || Store::open(&args.db, &args.seed))
        .await
        .context("database initialization task stopped")??;
    let mut service = Service {
        store,
        collector,
        slack,
    };
    match args.command {
        Command::Once => service.poll().await,
        Command::Run { interval_seconds } => {
            let shutdown = shutdown_signal();
            tokio::pin!(shutdown);
            loop {
                tokio::select! {
                    signal = &mut shutdown => return signal,
                    _result = service.poll() => {}
                }
                tokio::select! {
                    signal = &mut shutdown => return signal,
                    () = tokio::time::sleep(Duration::from_secs(interval_seconds)) => {}
                }
            }
        }
        Command::Status | Command::Backtest => Ok(()),
    }
}

#[cfg(unix)]
async fn shutdown_signal() -> Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate()).context("register SIGTERM handler")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.context("receive interrupt"),
        value = terminate.recv() => value.context("SIGTERM stream closed"),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> Result<()> {
    tokio::signal::ctrl_c().await.context("receive interrupt")
}
