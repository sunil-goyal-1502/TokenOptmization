use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokenopt_core::{
    analyze_trace, compile_context, AgentTrace, CompileOptions, FileColdStore, MemoryColdStore,
};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "ctxc", about = "TokenOpt context compiler CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Token breakdown by block kind
    Analyze {
        #[arg(short, long)]
        trace: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Compile transcript under budget with sufficiency gating
    Compile {
        #[arg(short, long)]
        trace: PathBuf,
        #[arg(short, long, default_value = "128000")]
        budget: u64,
        #[arg(short, long, default_value = "default")]
        session: String,
        #[arg(long)]
        store_dir: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        soft_sufficiency: bool,
        #[arg(long)]
        no_sufficiency: bool,
    },
    /// Validate trace JSON against schema expectations
    Validate {
        #[arg(short, long)]
        trace: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Analyze { trace, json } => cmd_analyze(trace, json).await,
        Commands::Compile {
            trace,
            budget,
            session,
            store_dir,
            output,
            soft_sufficiency,
            no_sufficiency,
        } => {
            cmd_compile(
                trace,
                budget,
                session,
                store_dir,
                output,
                soft_sufficiency,
                no_sufficiency,
            )
            .await
        }
        Commands::Validate { trace } => cmd_validate(trace).await,
    }
}

async fn load_trace(path: &PathBuf) -> anyhow::Result<AgentTrace> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    AgentTrace::from_json_slice(&bytes).context("parse trace JSON")
}

async fn cmd_analyze(trace: PathBuf, json: bool) -> anyhow::Result<()> {
    let agent_trace = load_trace(&trace).await?;
    let report = analyze_trace(&agent_trace.messages)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("total_tokens: {}", report.total_tokens);
        println!("block_count: {}", report.block_count);
        for (kind, tokens) in &report.tokens_by_kind {
            println!("  {kind}: {tokens}");
        }
    }
    Ok(())
}

async fn cmd_compile(
    trace: PathBuf,
    budget: u64,
    session: String,
    store_dir: Option<PathBuf>,
    output: Option<PathBuf>,
    soft_sufficiency: bool,
    no_sufficiency: bool,
) -> anyhow::Result<()> {
    let agent_trace = load_trace(&trace).await?;
    let store: Arc<dyn tokenopt_core::ColdStore> = match store_dir {
        Some(dir) => Arc::new(FileColdStore::new(dir)),
        None => Arc::new(MemoryColdStore::new()),
    };

    let opts = CompileOptions {
        session_id: session,
        token_budget: budget,
        soft_sufficiency,
        run_sufficiency_check: !no_sufficiency,
        ..Default::default()
    };

    let result = compile_context(&agent_trace.messages, opts, store, None).await?;

    let payload = serde_json::json!({
        "messages": result.messages,
        "stats": result.stats,
        "sufficient": result.sufficient,
        "sufficiency_message": result.sufficiency_message,
    });

    let out = serde_json::to_string_pretty(&payload)?;
    if let Some(path) = output {
        tokio::fs::write(&path, &out).await?;
        eprintln!("wrote {}", path.display());
    } else {
        println!("{out}");
    }

    eprintln!(
        "saved {} tokens ({:.1}%)",
        result.stats.tokens_saved, result.stats.reduction_percent
    );
    Ok(())
}

async fn cmd_validate(trace: PathBuf) -> anyhow::Result<()> {
    let agent_trace = load_trace(&trace).await?;
    tokenopt_core::validate_blocks(&tokenopt_core::parse_transcript(&agent_trace.messages)?)?;
    println!("valid: {} messages", agent_trace.messages.len());
    Ok(())
}
