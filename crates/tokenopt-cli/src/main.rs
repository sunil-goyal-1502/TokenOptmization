use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokenopt_core::{
    analyze_trace, bench_compile_latency, compare_trace, compile_context, simulate_agent_loop,
    AgentLoopSimConfig, AgentTrace, CompileOptions, FileColdStore, MemoryColdStore,
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
    /// Compare baseline vs TokenOpt on the same trace (tokens + latency)
    Compare {
        #[arg(short, long)]
        trace: PathBuf,
        #[arg(short, long, default_value = "128000")]
        budget: u64,
        #[arg(short, long, default_value = "default")]
        session: String,
        #[arg(long, default_value = "2")]
        keep_recent: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        no_sufficiency: bool,
    },
    /// Simulate multi-turn agent loop and measure token savings (no LLM)
    Bench {
        #[command(subcommand)]
        bench: BenchCommands,
    },
}

#[derive(Subcommand)]
enum BenchCommands {
    /// Compile latency percentiles (see also `ctxc bench latency`)
    Latency {
        #[arg(short, long)]
        trace: Option<PathBuf>,
        #[arg(long, default_value = "100")]
        iterations: u32,
        #[arg(long, default_value = "15")]
        sim_turns: u32,
        #[arg(long, default_value = "8000")]
        payload_bytes: usize,
        #[arg(long)]
        json: bool,
    },
    /// Simulate supervisor + workers; compare baseline / independent / MACO
    Orchestrator {
        #[arg(long, default_value = "4")]
        workers: u32,
        #[arg(long, default_value = "10")]
        rounds: u32,
        #[arg(long, default_value = "6000")]
        payload_bytes: usize,
        /// Fraction of worker reads per round hitting shared artifacts (0..1)
        #[arg(long, default_value = "0.5")]
        shared_fraction: f64,
        #[arg(long, default_value = "64000")]
        global_budget: u64,
        #[arg(long, default_value = "2")]
        keep_recent: usize,
        #[arg(long)]
        json: bool,
    },
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
        Commands::Compare {
            trace,
            budget,
            session,
            keep_recent,
            json,
            no_sufficiency,
        } => {
            cmd_compare(trace, budget, session, keep_recent, json, no_sufficiency).await
        }
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
            BenchCommands::Latency {
                trace,
                iterations,
                sim_turns,
                payload_bytes,
                json,
            } => cmd_bench_latency(trace, iterations, sim_turns, payload_bytes, json).await,
            BenchCommands::Orchestrator {
                workers,
                rounds,
                payload_bytes,
                shared_fraction,
                global_budget,
                keep_recent,
                json,
            } => {
                cmd_bench_orchestrator(
                    workers,
                    rounds,
                    payload_bytes,
                    shared_fraction,
                    global_budget,
                    keep_recent,
                    json,
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
        "saved {} tokens ({:.1}%) | compile {}ms (transform {}ms, oracle {}ms) | cold_refs {}",
        result.stats.tokens_saved,
        result.stats.reduction_percent,
        result.stats.compile_duration_ms,
        result.stats.transform_duration_ms,
        result.stats.oracle_duration_ms,
        result.stats.cold_refs_count,
    );
    Ok(())
}

async fn cmd_compare(
    trace: PathBuf,
    budget: u64,
    session: String,
    keep_recent: usize,
    json: bool,
    no_sufficiency: bool,
) -> anyhow::Result<()> {
    let agent_trace = load_trace(&trace).await?;
    let store = Arc::new(MemoryColdStore::new());
    let report = compare_trace(
        &agent_trace.messages,
        CompileOptions {
            session_id: session,
            token_budget: budget,
            keep_recent_tool_results: keep_recent,
            run_sufficiency_check: !no_sufficiency,
            ..Default::default()
        },
        store,
    )
    .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("TokenOpt compare (same trace, baseline vs compiled)");
    println!();
    println!("Tokens:");
    println!("  baseline:   {}", report.baseline_tokens);
    println!("  compiled:   {}", report.compiled_tokens);
    println!("  saved:      {} ({:.1}%)", report.tokens_saved, report.reduction_percent);
    println!();
    println!("Message JSON size:");
    println!("  baseline:   {} chars", report.baseline_message_chars);
    println!("  compiled:   {} chars", report.compiled_message_chars);
    println!("  reduction:  {:.1}%", report.char_reduction_percent);
    println!();
    println!("Compiler latency (this run):");
    println!("  total:      {} ms", report.compile_duration_ms);
    println!("  transform:  {} ms", report.transform_duration_ms);
    println!("  oracle:     {} ms", report.oracle_duration_ms);
    println!("  cold refs:  {}", report.cold_refs_count);
    println!("  sufficient: {}", report.sufficient);
    Ok(())
}

async fn cmd_bench_latency(
    trace: Option<PathBuf>,
    iterations: u32,
    sim_turns: u32,
    payload_bytes: usize,
    json: bool,
) -> anyhow::Result<()> {
    let messages = if let Some(path) = trace {
        load_trace(&path).await?.messages
    } else {
        tokenopt_core::generate_agent_trace(&AgentLoopSimConfig {
            turns: sim_turns,
            tool_payload_bytes: payload_bytes,
            ..Default::default()
        })
    };

    let store = Arc::new(MemoryColdStore::new());
    let report = bench_compile_latency(
        &messages,
        CompileOptions {
            session_id: "latency-bench".into(),
            run_sufficiency_check: false,
            ..Default::default()
        },
        store,
        iterations,
    )
    .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Compile latency bench ({} iterations, {} messages)", iterations, report.trace_messages);
    println!("  p50:  {} ms", report.p50_ms);
    println!("  p95:  {} ms", report.p95_ms);
    println!("  p99:  {} ms", report.p99_ms);
    println!("  min:  {} ms", report.min_ms);
    println!("  max:  {} ms", report.max_ms);
    println!("  mean: {:.1} ms", report.mean_ms);
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

async fn cmd_bench_orchestrator(
    workers: u32,
    rounds: u32,
    payload_bytes: usize,
    shared_fraction: f64,
    global_budget: u64,
    keep_recent: usize,
    json: bool,
) -> anyhow::Result<()> {
    let config = tokenopt_core::OrchestratorSimConfig {
        workers,
        rounds,
        tool_payload_bytes: payload_bytes,
        shared_read_fraction: shared_fraction.clamp(0.0, 1.0),
        global_token_budget: global_budget,
        keep_recent_tool_results: keep_recent,
    };
    let report = tokenopt_core::simulate_orchestrator_loop(config).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    eprintln!("MACO orchestrator benchmark (synthetic, no LLM)");
    eprintln!(
        "agents: 1 supervisor + {} workers | rounds: {} | shared reads: {:.0}% | global budget: {}",
        workers,
        rounds,
        shared_fraction * 100.0,
        global_budget
    );
    eprintln!();
    eprintln!(
        "{:>5} {:>12} {:>12} {:>12} {:>8} {:>14} {:>14}",
        "round", "baseline", "independent", "maco", "dedup#", "vs-baseline", "vs-independent"
    );
    for r in &report.rounds {
        eprintln!(
            "{:>5} {:>12} {:>12} {:>12} {:>8} {:>13.1}% {:>13.1}%",
            r.round,
            r.baseline_tokens,
            r.independent_tokens,
            r.maco_tokens,
            r.dedup_duplicates_masked,
            r.maco_vs_baseline_percent,
            r.maco_vs_independent_percent
        );
    }
    eprintln!();
    eprintln!("FINAL round {}:", report.rounds.len());
    eprintln!("  baseline tokens:     {}", report.final_baseline_tokens);
    eprintln!("  independent tokens:  {}", report.final_independent_tokens);
    eprintln!("  maco tokens:         {}", report.final_maco_tokens);
    eprintln!(
        "  maco vs baseline:    {:.1}% saved",
        report.final_maco_vs_baseline_percent
    );
    eprintln!(
        "  maco vs independent: {:.1}% saved",
        report.final_maco_vs_independent_percent
    );
    eprintln!();
    eprintln!("CUMULATIVE (sum across rounds):");
    eprintln!("  baseline:    {}", report.cumulative_baseline_tokens);
    eprintln!("  independent: {}", report.cumulative_independent_tokens);
    eprintln!("  maco:        {}", report.cumulative_maco_tokens);
    Ok(())
}

async fn cmd_validate(trace: PathBuf) -> anyhow::Result<()> {
    let agent_trace = load_trace(&trace).await?;
    tokenopt_core::validate_blocks(&tokenopt_core::parse_transcript(&agent_trace.messages)?)?;
    println!("valid: {} messages", agent_trace.messages.len());
    Ok(())
}
