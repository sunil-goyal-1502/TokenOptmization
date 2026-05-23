use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokenopt_core::{
    analyze_trace, compile_context, simulate_agent_loop, AgentLoopSimConfig, AgentTrace,
    CompileOptions, FileColdStore, MemoryColdStore,
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
    /// Simulate multi-turn agent loop and measure token savings (no LLM)
    Bench {
        #[command(subcommand)]
        bench: BenchCommands,
    },
}

#[derive(Subcommand)]
enum BenchCommands {
    /// Run synthetic agent loop; compiles context before each turn
    AgentLoop {
        #[arg(long, default_value = "20")]
        turns: u32,
        #[arg(long, default_value = "8000")]
        payload_bytes: usize,
        #[arg(long, default_value = "2")]
        keep_recent: usize,
        #[arg(long, default_value = "128000")]
        budget: u64,
        #[arg(long)]
        no_masking: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        write_trace: Option<PathBuf>,
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
        Commands::Bench { bench } => match bench {
            BenchCommands::AgentLoop {
                turns,
                payload_bytes,
                keep_recent,
                budget,
                no_masking,
                json,
                write_trace,
            } => {
                cmd_bench_agent_loop(
                    turns,
                    payload_bytes,
                    keep_recent,
                    budget,
                    no_masking,
                    json,
                    write_trace,
                )
                .await
            }
        },
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

async fn cmd_bench_agent_loop(
    turns: u32,
    payload_bytes: usize,
    keep_recent: usize,
    budget: u64,
    no_masking: bool,
    json: bool,
    write_trace: Option<PathBuf>,
) -> anyhow::Result<()> {
    let config = AgentLoopSimConfig {
        turns,
        tool_payload_bytes: payload_bytes,
        keep_recent_tool_results: keep_recent,
        token_budget: budget,
        enable_consumed_masking: !no_masking,
        run_sufficiency_check: false,
    };

    if let Some(path) = write_trace {
        let messages = tokenopt_core::generate_agent_trace(&config);
        let trace = AgentTrace {
            trace_id: "bench".into(),
            session_id: "bench-session".into(),
            messages,
            metadata: Some(tokenopt_core::TraceMetadata {
                orchestrator: Some("ctxc-bench".into()),
                model: None,
                task_id: None,
            }),
        };
        tokio::fs::write(&path, serde_json::to_string_pretty(&trace)?).await?;
        eprintln!("wrote trace {}", path.display());
    }

    let report = simulate_agent_loop(config).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    eprintln!("TokenOpt agent-loop benchmark (synthetic, no LLM)");
    eprintln!("Active optimizations: {}", report.optimizations_active.join(", "));
    eprintln!();
    eprintln!(
        "{:>4} {:>6} {:>12} {:>12} {:>10} {:>8}",
        "turn", "msgs", "baseline", "compiled", "saved", "reduct%"
    );
    for t in &report.turns {
        if t.turn % 5 == 0 || t.turn == report.turns.len() as u32 {
            eprintln!(
                "{:>4} {:>6} {:>12} {:>12} {:>10} {:>7.1}%",
                t.turn,
                t.message_count,
                t.baseline_tokens,
                t.compiled_tokens,
                t.tokens_saved,
                t.reduction_percent
            );
        }
    }
    eprintln!();
    eprintln!("FINAL turn {}:", report.turns.last().map(|t| t.turn).unwrap_or(0));
    eprintln!("  baseline tokens:  {}", report.final_baseline_tokens);
    eprintln!("  compiled tokens:  {}", report.final_compiled_tokens);
    eprintln!(
        "  reduction:        {:.1}%",
        report.final_reduction_percent
    );
    eprintln!(
        "  cumulative saved: {} tokens (sum per-turn deltas)",
        report.total_tokens_saved_across_turns
    );
    Ok(())
}

async fn cmd_validate(trace: PathBuf) -> anyhow::Result<()> {
    let agent_trace = load_trace(&trace).await?;
    tokenopt_core::validate_blocks(&tokenopt_core::parse_transcript(&agent_trace.messages)?)?;
    println!("valid: {} messages", agent_trace.messages.len());
    Ok(())
}
