use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use tokenopt_core::{
    analyze_trace, collapse_branch_messages, compare_trace, compile_context, compile_multi_agent,
    metrics_snapshot, prometheus_text, rehydrate_messages, AgentContext, AgentMiddleware,
    CompareReport, CompileOptions, CompileResult, FileColdStore, FoldRecord,
    MultiAgentCompileResult, MultiAgentOptions, OrchestratorAdapter, RehydrateOptions,
    TranscriptMessage,
};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "tokenopt-server", about = "HTTP API for TokenOpt context compiler")]
struct Args {
    #[arg(short, long, default_value = "127.0.0.1:8787")]
    bind: String,
    #[arg(long, default_value = ".ctxc-store")]
    store_dir: PathBuf,
}

#[derive(Clone)]
struct AppState {
    store: Arc<dyn tokenopt_core::ColdStore>,
    default_budget: u64,
}

#[derive(Debug, Deserialize)]
struct CompileRequest {
    pub messages: Vec<TranscriptMessage>,
    #[serde(default)]
    pub options: CompileOptions,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    pub error: String,
}

#[derive(Debug, Deserialize)]
struct AnalyzeRequest {
    pub messages: Vec<TranscriptMessage>,
}

#[derive(Debug, Deserialize)]
struct MiddlewareRequest {
    pub messages: Vec<TranscriptMessage>,
    pub session_id: String,
    #[serde(default)]
    pub turn_index: u32,
    #[serde(default)]
    pub options: CompileOptions,
}

#[derive(Debug, Deserialize)]
struct RehydrateRequest {
    pub messages: Vec<TranscriptMessage>,
    #[serde(default)]
    pub options: RehydrateOptions,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    tokio::fs::create_dir_all(&args.store_dir).await?;

    let state = AppState {
        store: Arc::new(FileColdStore::new(&args.store_dir)),
        default_budget: 128_000,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/analyze", post(analyze))
        .route("/v1/compile", post(compile))
        .route("/v1/middleware/before-model", post(before_model))
        .route("/v1/rehydrate", post(rehydrate))
        .route("/v1/compare", post(compare))
        .route("/v1/fold/collapse", post(fold_collapse))
        .route("/v1/orchestrator/compile", post(orchestrator_compile))
        .route("/v1/metrics", get(metrics))
        .route("/v1/metrics/prometheus", get(metrics_prometheus))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = args.bind.parse()?;
    tracing::info!("tokenopt-server listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "service": "tokenopt-server" }))
}

async fn analyze(
    State(state): State<AppState>,
    Json(req): Json<AnalyzeRequest>,
) -> Result<Json<tokenopt_core::AnalyzeReport>, AppError> {
    let _ = state;
    let report = analyze_trace(&req.messages)?;
    Ok(Json(report))
}

async fn compile(
    State(state): State<AppState>,
    Json(req): Json<CompileRequest>,
) -> Result<Json<CompileResult>, AppError> {
    let mut opts = req.options;
    if opts.token_budget == 0 {
        opts.token_budget = state.default_budget;
    }
    if opts.session_id.is_empty() {
        opts.session_id = uuid::Uuid::new_v4().to_string();
    }
    let result = compile_context(&req.messages, opts, state.store.clone(), None).await?;
    Ok(Json(result))
}

async fn metrics() -> impl IntoResponse {
    Json(metrics_snapshot())
}

async fn metrics_prometheus() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        prometheus_text(),
    )
}

async fn rehydrate(
    State(state): State<AppState>,
    Json(req): Json<RehydrateRequest>,
) -> Result<Json<tokenopt_core::RehydrateResult>, AppError> {
    let result = rehydrate_messages(&req.messages, state.store.clone(), req.options).await?;
    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
struct FoldCollapseRequest {
    pub messages: Vec<TranscriptMessage>,
}

#[derive(Debug, Serialize)]
struct FoldCollapseResponse {
    pub messages: Vec<TranscriptMessage>,
    pub fold_records: Vec<FoldRecord>,
}

async fn fold_collapse(
    Json(req): Json<FoldCollapseRequest>,
) -> Result<Json<FoldCollapseResponse>, AppError> {
    let (messages, fold_records) = collapse_branch_messages(&req.messages)?;
    Ok(Json(FoldCollapseResponse {
        messages,
        fold_records,
    }))
}

#[derive(Debug, Deserialize)]
struct OrchestratorCompileRequest {
    pub agents: Vec<AgentContext>,
    #[serde(default)]
    pub options: MultiAgentOptions,
}

/// Compile every agent's context under one shared global budget (MACO):
/// cross-agent dedup, water-filling allocation, per-agent compilation.
async fn orchestrator_compile(
    State(state): State<AppState>,
    Json(req): Json<OrchestratorCompileRequest>,
) -> Result<Json<MultiAgentCompileResult>, AppError> {
    let mut opts = req.options;
    if opts.global_token_budget == 0 {
        opts.global_token_budget = state.default_budget;
    }
    let result = compile_multi_agent(&req.agents, opts, state.store.clone()).await?;
    Ok(Json(result))
}

async fn compare(
    State(state): State<AppState>,
    Json(req): Json<CompileRequest>,
) -> Result<Json<CompareReport>, AppError> {
    let mut opts = req.options;
    if opts.token_budget == 0 {
        opts.token_budget = state.default_budget;
    }
    if opts.session_id.is_empty() {
        opts.session_id = uuid::Uuid::new_v4().to_string();
    }
    let report = compare_trace(&req.messages, opts, state.store.clone()).await?;
    Ok(Json(report))
}

async fn before_model(
    State(state): State<AppState>,
    Json(req): Json<MiddlewareRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut opts = req.options;
    opts.session_id = req.session_id.clone();
    let middleware = AgentMiddleware::new(opts, state.store.clone());
    let ctx = tokenopt_core::MiddlewareContext {
        session_id: req.session_id,
        turn_index: req.turn_index,
        orchestrator: None,
        metadata: None,
    };
    let messages = middleware.on_before_model(&req.messages, &ctx).await?;
    Ok(Json(serde_json::json!({ "messages": messages })))
}

struct AppError(tokenopt_core::CompilerError);

impl From<tokenopt_core::CompilerError> for AppError {
    fn from(e: tokenopt_core::CompilerError) -> Self {
        AppError(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let body = ErrorBody {
            error: self.0.to_string(),
        };
        (StatusCode::BAD_REQUEST, Json(body)).into_response()
    }
}
