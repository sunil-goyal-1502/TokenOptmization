# AGENTS

## Cursor Cloud specific instructions

Rust workspace (`tokenopt-core`, `tokenopt-server`, `tokenopt-cli`) plus Python/TypeScript HTTP clients.

### Quick commands

```bash
cargo build --workspace
cargo test -p tokenopt-core
cargo run -p tokenopt-server -- --bind 127.0.0.1:8787
```

### E2E (optional)

```bash
pip install -r examples/e2e/requirements.txt
pip install -e bindings/python
./scripts/run_swe_bench.sh
```

### Secrets

- Never commit `.env` or API keys. Use `OPENAI_API_KEY` from the environment.
- Default server bind is **localhost only** (`127.0.0.1:8787`). See `docs/SECURITY.md`.

### Services

| Service | Port | Notes |
|---------|------|-------|
| `tokenopt-server` | 8787 | Context compiler HTTP API |
| `compress_sidecar` | 8790 | Optional; `python3 examples/compress_sidecar.py` |
