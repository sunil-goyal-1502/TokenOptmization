# Metrics, latency, and quality measurement

## Compiler stats (every `/v1/compile` response)

```json
{
  "stats": {
    "input_tokens": 51000,
    "output_tokens": 6400,
    "tokens_saved": 44600,
    "reduction_percent": 87.5,
    "compile_duration_ms": 12,
    "transform_duration_ms": 10,
    "oracle_duration_ms": 1,
    "cold_refs_count": 8
  }
}
```

| Field | Meaning |
|-------|---------|
| `compile_duration_ms` | Wall-clock for full compile |
| `transform_duration_ms` | Transform pipeline only |
| `oracle_duration_ms` | Sufficiency check only |
| `cold_refs_count` | Blocks pointing at cold store |

## Server metrics

```bash
curl http://127.0.0.1:8787/v1/metrics
curl http://127.0.0.1:8787/v1/metrics/prometheus
```

Tracks: request count, errors, avg/max/last latency, histogram buckets (≤1ms … >200ms).

## CLI

```bash
# Same trace: tokens + latency breakdown
ctxc compare --trace fixtures/generated-long-trace.json

# Compile latency p50/p95/p99 (synthetic long trace if no file)
ctxc bench latency --iterations 200 --sim-turns 20 --payload-bytes 12000
```

## Rehydrate masked content

```bash
curl -X POST http://127.0.0.1:8787/v1/rehydrate \
  -H 'Content-Type: application/json' \
  -d '{"messages":[...], "options":{"max_bytes_per_ref":65536}}'
```

Use when the agent must read full tool output after masking.

## Quality + end-to-end latency (your orchestrator)

TokenOpt only measures **compiler** latency today. For production:

1. Log per turn: `compile_duration_ms`, `usage.prompt_tokens`, `llm_duration_ms`, `task_step`.
2. A/B middleware on/off on the same task suite.
3. Score: `success_rate`, `median_task_wall_clock`, `total_tokens_per_task`.

See `examples/e2e/run_real_agent_e2e.py` for real LLM + real tools.

## Interpreting tradeoffs

| Observation | Likely cause |
|-------------|--------------|
| High `tokens_saved`, flat success | Healthy |
| High `tokens_saved`, success drops | Masking too aggressive → add rehydrate or raise `keep_recent_tool_results` |
| High `compile_duration_ms` vs low LLM time | OK if compile &lt;5% of turn time |
| Higher total tokens despite masking | Agent re-fetches refs in extra turns |
