# Full-stack E2E test report (2026-05-24)

Environment: Ollama `tinyllama`, no `OPENAI_API_KEY`.

## Summary

| Component | Status | Notes |
|-----------|--------|-------|
| tokenopt-server HTTP | PASS | health, compile, compare, rehydrate, fold, metrics |
| Compress sidecar :8790 | PASS | 800 chars → 179 chars stub compression |
| Research pipeline HTTP | PASS | 88.2% reduction on long trace; all transforms in list |
| Real agent (default opts) | PASS | 19.1% char reduction (3 turns) |
| Real agent (`--research` + sidecar) | PASS | **86.6%** char reduction (3 turns) |
| Quality A/B (3 tasks) | PASS | All baseline + tokenopt runs `success=True` |
| SWE-bench harness (Rust) | PASS | 3/3 traces meet reduction targets |
| Node native (napi) | PASS | `compileJson` in-process |
| Python native (pyo3) | PASS* | Works when `_native.so` is in `tokenopt/` package |

\* Editable `pip install -e bindings/python` does not bundle the `.so`; use `maturin build` + install wheel or copy artifact.

## Research pipeline transforms observed (HTTP compile)

`guideline_pin`, `fold_policy`, `fold_collapse`, `memory_prune`, `rule_summarize`, `external_compress`, `agent_omit`, `cache_packer`, plus core masking/trim.

## Not tested in this run

- OpenAI provider (no API key in VM)
- LLMLingua GPU (`pip install llmlingua`) — sidecar used **stub** compressor
- LLM summarization / LLM oracle (`llm-http` + API key)
- Full SWE-bench Docker patch evaluation
- Automatic model routing from `routing_hint`

## Reproduce

```bash
./scripts/run_full_stack_e2e.sh
# or manually:
cargo run --release -p tokenopt-server -- --bind 127.0.0.1:8787 &
python3 examples/compress_sidecar.py &
python3 examples/e2e/run_real_agent_e2e.py --provider ollama --turns 4 --research
```
