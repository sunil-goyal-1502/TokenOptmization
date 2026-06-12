# Research backlog — implementation status

All major research items now have code paths in this repository.

## Implemented

| Item | Location |
|------|----------|
| SGCC pipeline | `compile.rs`, `pipeline.rs` |
| ACON guideline bank | `guideline.rs`, `fixtures/guidelines/` |
| Context folding | `fold.rs`, `fold_runtime.rs`, `role: fold` |
| Fold policy (JSON / GRPO scores export) | `fold_policy.rs`, `fixtures/fold_policies/` |
| Rule + LLM summarization | `rule_summarize`, `llm_summarize` |
| BACM critical memory | `bacm.rs` |
| LLMLingua-2 hook | `external_compress` + `examples/compress_sidecar.py` |
| Routing hints | `routing.rs` |
| LLM sufficiency oracle | `llm.rs` + `llm-http` feature |
| Auto rehydrate | `CompileOptions.auto_rehydrate_refs` |
| Accurate tokens | `accurate-tokens` feature |
| **Native Python (pyo3)** | `crates/tokenopt-py` → `tokenopt._native` |
| **Native Node (napi-rs)** | `crates/tokenopt-node` → `@tokenopt/native` |
| **SWE-bench-lite harness** | `fixtures/swe_bench/`, `tests/swe_bench_harness.rs` |
| Publish metadata | `docs/PUBLISHING.md`, crate `publish = true` on core |
| **MACO multi-agent orchestration** | `orchestrator.rs`, `orchestrator_sim.rs`, [`docs/MULTI_AGENT_RESEARCH.md`](MULTI_AGENT_RESEARCH.md) |

## Operational limits

- **FoldGRPO training** — not in-repo; export scores to `fixtures/fold_policies/*.json`
- **Full SWE-bench Docker eval** — use upstream SWE-bench; we validate **compiler reduction** on traces
- **LLMLingua-2** — optional Python dep on sidecar; stub fallback without GPU

## Quick commands

```bash
cargo test -p tokenopt-core --test swe_bench_harness
./scripts/run_swe_bench.sh

# Native Python
pip install maturin && cd bindings/python && maturin develop --release

# Native Node
cd bindings/node && npm install && npm run build
```
