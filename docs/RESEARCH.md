# Research backlog → implementation status

## Implemented in `tokenopt-core`

| Research item | Module / transform | Enable |
|---------------|-------------------|--------|
| SGCC compile pipeline | `compile.rs`, `pipeline.rs` | always |
| ACON guideline bank | `guideline.rs` → `guideline_pin` | `transforms.guideline_bank` |
| Context folding | `fold.rs`, `fold_runtime.rs`, `fold_inject`, `fold_collapse` | `fold_records`, `role: fold` messages |
| Rule summarization | `rule_summarize` | `transforms.summarization` (default on) |
| Prompt-cache packer | `cache_packer` | `transforms.cache_packer` (default on) |
| Agent-Omit | `agent_omit` | `transforms.agent_omit` (default on) |
| MEM1-style prune | `memory_prune` | `transforms.memory_prune` |
| LLMLingua-2 hook | `external_compress` + `examples/compress_sidecar.py` | `external_compress_url` |
| Cascade routing hint | `routing.rs` | `routing_hints` on `CompileResult` |
| LLM sufficiency oracle | `llm.rs` | `llm_oracle.enabled` + `--features llm-http` |
| Smart rehydrate | `rehydrate.rs` | `auto_rehydrate_refs` |
| Accurate tokens | `tokens.rs` | `--features accurate-tokens` |

## Still out of scope

- Trained folding policy (FoldGRPO / RL)
- In-process pyo3 / napi (HTTP clients only)
- crates.io / PyPI / npm publish
- SWE-bench-scale automated quality CI
- Full BACM paper reproduction (partial via `memory_prune` + `agent_omit`)

## Example: enable research pipeline

```json
{
  "options": {
    "session_id": "run-1",
    "transforms": {
      "guideline_bank": true,
      "memory_prune": true
    },
    "guideline_bank_path": "fixtures/guidelines/default.json",
    "fold_records": [{"subgoal": "setup", "status": "success"}],
    "external_compress_url": "http://127.0.0.1:8790/compress",
    "routing_hints": true,
    "auto_rehydrate_refs": false
  }
}
```

Build with LLM oracle:

```bash
cargo build -p tokenopt-core --features llm-http
export OPENAI_API_KEY=sk-...
# CompileOptions.llm_oracle = { "enabled": true }
```
