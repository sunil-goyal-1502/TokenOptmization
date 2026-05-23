# Testing TokenOpt

Token optimization is measured as **fewer tokens sent to the model** on each turn (estimated via `chars/4` heuristic, or your orchestrator’s tokenizer in production).

## What optimizations are implemented?

| Transform | What it does | When you see savings |
|-----------|--------------|----------------------|
| **consumed_result_mask** | Replaces old `tool` results with a short preview + `ref://` cold-store pointer | Multi-turn agents with large `read_file` / shell / MCP outputs (main lever) |
| **error_compaction** | Shrinks old failed tool results after N turns | Long runs with repeated test/lint failures |
| **referential_keep** | Tracks paths/symbols in recent turns for sufficiency | Supports oracle; minor alone |
| **budget_trim** | Drops middle blocks when over `token_budget` | Very long traces exceeding budget |
| **sufficiency oracle** | Rule-based check that required slots still appear in context | Quality gate (can block or soft-fail) |
| **rolling_window** | Drops middle blocks when over budget; inserts summary marker | Traces exceeding `token_budget` after masking |

**Not implemented yet:** LLM summarization, ACON guideline bank, prompt-cache layout, model routing.

---

## Level 1 — Unit / integration tests (no agents, no API keys)

```bash
cargo test --workspace
```

Includes:

- `integration.rs` — parse + compile on fixture
- `agent_loop_sim.rs` — synthetic 15-turn loop must show **>10%** token reduction with masking on

---

## Level 2 — Synthetic agent loop benchmark (recommended)

Simulates a coding agent that appends a large tool result every turn, then runs **compile before each model call** (same as production middleware).

```bash
cargo build -p tokenopt-cli

# Default: 20 turns, 8KB per tool result, keep 2 recent results
cargo run -p tokenopt-cli -- bench agent-loop

# Heavier load (closer to real SWE agents)
cargo run -p tokenopt-cli -- bench agent-loop \
  --turns 30 \
  --payload-bytes 16000 \
  --keep-recent 2

# A/B: disable masking (should show lower savings)
cargo run -p tokenopt-cli -- bench agent-loop --turns 20 --no-masking

# JSON for dashboards + write trace for ctxc compile
cargo run -p tokenopt-cli -- bench agent-loop --json \
  --write-trace fixtures/generated-long-trace.json
```

**What to look for:** `FINAL` line — `reduction` should grow with turn count (e.g. 30–70% on turn 20+ with large payloads).

---

## Level 3 — Trace file (recorded or synthetic)

```bash
# Analyze breakdown
cargo run -p tokenopt-cli -- analyze --trace fixtures/sample-trace.json

# Compile and print stats
cargo run -p tokenopt-cli -- compile --trace fixtures/generated-long-trace.json \
  --budget 32000 --soft-sufficiency
```

Use a **real** trace exported from your orchestrator (OpenAI-style `messages[]` JSON). See `schemas/trace.schema.json`.

---

## Level 4 — HTTP server + orchestrator hook (real agent integration)

### 1. Start server

```bash
cargo run -p tokenopt-server -- --bind 127.0.0.1:8787
```

### 2. Wire `before-model` in your agent

Before every LLM call, POST compiled messages:

```bash
curl -s http://127.0.0.1:8787/v1/middleware/before-model \
  -H 'Content-Type: application/json' \
  -d '{
    "session_id": "run-1",
    "turn_index": 5,
    "messages": [...],
    "options": {
      "token_budget": 128000,
      "keep_recent_tool_results": 3,
      "soft_sufficiency": true
    }
  }'
```

Log `stats` from `/v1/compile` if you call compile directly.

### 3. Python simulated loop (server required)

```bash
pip install -e bindings/python
cargo run -p tokenopt-server -- --bind 127.0.0.1:8787 &
python examples/test_agent_loop.py
```

### 4. Real LLM agent (Cursor, LangGraph, custom)

| Step | Action |
|------|--------|
| 1 | Run `tokenopt-server` locally or in your cluster |
| 2 | In harness: `messages = client.before_model(messages, session_id)` |
| 3 | Log **provider** `usage.input_tokens` with vs without middleware |
| 4 | Compare task success rate (must not regress) |

**Cursor:** use `examples/cursor-hook.mjs` in a Hook that calls the server (see Cursor hooks docs).

---

## Level 5 — Real LLM agent E2E (actual model + real tools)

This is **not** the synthetic `bench agent-loop`. It calls a **real model** (OpenAI or Ollama) and runs **real** `read_file` / `grep` / `list_dir` on this repo.

### Prerequisites

```bash
# 1. TokenOpt server
cargo run -p tokenopt-server -- --bind 127.0.0.1:8787

# 2. LLM provider (pick one)
export OPENAI_API_KEY=sk-...          # OpenAI — gpt-4o-mini, native tools
# OR
ollama serve && ollama pull tinyllama  # Local — free, no API key
export TOKENOPT_E2E_PROVIDER=ollama

# 3. Python deps
pip install -r examples/e2e/requirements.txt
pip install -e bindings/python
```

### Run

```bash
./scripts/run_real_agent_e2e.sh --turns 6 --padding-kb 6
# or
python3 examples/e2e/run_real_agent_e2e.py --provider ollama --turns 6 --padding-kb 6
```

**What it measures:** On each turn, before the LLM call, it compares message JSON size **with vs without** `POST /v1/compile` on the **same** transcript. That is what your orchestrator would bill as input tokens (use provider `usage.prompt_tokens` when on OpenAI).

**Example (Ollama + tinyllama, 5 turns, 6KB padding per tool):**

```
reduction: 51.0%
turn 5 list_dir  base_chars 43796  opt_chars 8721  saved 80.1%
```

### OpenAI (recommended for production-like metrics)

```bash
export OPENAI_API_KEY=sk-...
python3 examples/e2e/run_real_agent_e2e.py --provider openai --model gpt-4o-mini --turns 8
```

Use `--dual-run` only if you want two separate full agent runs (slower; small models are non-deterministic so API totals may not decrease even when context shrinks).

---

## Level 5b — Compare & latency (same trace)

```bash
# Generate long trace
cargo run -p tokenopt-cli -- bench agent-loop --turns 20 --write-trace /tmp/long.json

# Tokens + compiler latency on identical input
cargo run -p tokenopt-cli -- compare --trace /tmp/long.json --keep-recent 2

# p50/p95/p99 compile time
cargo run -p tokenopt-cli -- bench latency --iterations 100 --sim-turns 20
```

Server metrics: `GET /v1/metrics` — see [METRICS.md](METRICS.md).

HTTP compare (same as `ctxc compare`):

```bash
curl -s http://127.0.0.1:8787/v1/compare \
  -H 'Content-Type: application/json' \
  -d '{"messages": [...], "options": {"keep_recent_tool_results": 2}}'
```

---

## Level 5c — Quality A/B (task success + tokens + latency)

Runs each task in `examples/e2e/quality_tasks.json` twice (baseline vs TokenOpt) with the **same** planned tool steps.

```bash
export OPENAI_API_KEY=sk-...   # or use .env at repo root (not committed)
./scripts/run_quality_ab.sh --provider openai
# or
python3 examples/e2e/run_quality_ab.py --provider openai
```

Output: `examples/e2e/quality_ab_report.json` — per-task success, `usage.prompt_tokens`, `compile_duration_ms`, wall-clock.

`.env` is loaded automatically via `python-dotenv` when installed.

---

## Level 6 — Production validation checklist

- [ ] **Token delta:** input tokens ↓ ≥20% on 20+ turn coding tasks  
- [ ] **Quality:** task success rate within ~2% of baseline  
- [ ] **Latency:** `before-model` p95 <200ms (rule oracle only)  
- [ ] **Recovery:** agent can `read` cold-store refs when it needs full tool output  
- [ ] **A/B:** `--no-masking` shows measurably worse savings (proves mask is doing work)

---

## Honest limitations today

- Token counts are **estimates** (`chars/4`) unless you build with `--features accurate-tokens` on `tokenopt-core`.  
- Small fixtures (~6 messages) show **0%** savings — you need long traces or `bench agent-loop`.  
- No end-to-end test with OpenAI/Anthropic billing yet — that’s your orchestrator + API keys.
