# TokenOpt

Production-ready **sufficiency-gated context compiler** for LLM agents. Reduces token usage without dropping task-critical state.

## Why Rust core?

| Layer | Role |
|-------|------|
| **`tokenopt-core`** (Rust) | IR, transforms, cold store, sufficiency oracle, compile pipeline |
| **`tokenopt-server`** | HTTP API — integrate from **any** language/orchestrator |
| **`ctxc`** CLI | Analyze, compile, validate traces |
| **Python / TypeScript clients** | Thin production SDKs over HTTP |

Rust gives a single correct implementation with predictable latency; Python and TypeScript cover LangGraph, CrewAI, Cursor hooks, and Node orchestrators without duplicating compiler logic.

## Quick start

```bash
# Build
cargo build --release

# Analyze a trace
cargo run -p tokenopt-cli -- analyze --trace fixtures/sample-trace.json

# Compile under budget
cargo run -p tokenopt-cli -- compile --trace fixtures/sample-trace.json --budget 32000

# Start HTTP server (for Python/TS/other orchestrators)
cargo run -p tokenopt-server -- --bind 127.0.0.1:8787
```

### Python (LangGraph, AutoGen, etc.)

```bash
pip install -e bindings/python
```

```python
from tokenopt import TokenOptClient, CompileOptions

with TokenOptClient() as client:
    out = client.compile(
        [{"role": "user", "content": "Fix src/lib.rs"}],
        CompileOptions(token_budget=80_000, session_id="job-1"),
    )
    print(out.stats.reduction_percent)
```

### TypeScript (Cursor hooks, LangChain.js, etc.)

```bash
cd bindings/typescript && npm install && npm run build
```

```typescript
import { TokenOptClient } from "@tokenopt/client";

const client = new TokenOptClient({ baseUrl: "http://127.0.0.1:8787" });
const result = await client.compile(messages, { token_budget: 80_000 });
```

## Orchestrator integration

Call **`POST /v1/middleware/before-model`** before every LLM invocation:

```json
{
  "messages": [...],
  "session_id": "run-abc",
  "turn_index": 12,
  "options": { "token_budget": 128000, "keep_recent_tool_results": 3 }
}
```

Or embed in Rust:

```rust
use std::sync::Arc;
use tokenopt_core::{compile_context, CompileOptions, MemoryColdStore};

let store = Arc::new(MemoryColdStore::new());
let result = compile_context(&messages, CompileOptions::default(), store, None).await?;
```

## Compiler pipeline

1. **Parse** transcript → typed IR (`system`, `tool_result`, …)
2. **Transform** — referential keep, error compaction, consumed-result masking, budget trim
3. **Sufficiency oracle** — verify required slots still present (or soft-fail / passthrough)
4. **Emit** compiled messages + stats + cold-store refs (`ref://session/key`)

## HTTP API

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Liveness |
| `POST /v1/analyze` | Token breakdown by block kind |
| `POST /v1/compile` | Full compile with stats |
| `POST /v1/middleware/before-model` | Drop-in pre-LLM hook |

## Schemas

- [`schemas/trace.schema.json`](schemas/trace.schema.json)
- [`schemas/fold-record.schema.json`](schemas/fold-record.schema.json)

## Project layout

```
crates/tokenopt-core/     # Rust library
crates/tokenopt-cli/      # ctxc CLI
crates/tokenopt-server/   # HTTP sidecar
bindings/python/          # PyPI-ready client
bindings/typescript/      # npm client
schemas/                  # JSON Schema contracts
fixtures/                 # Sample traces
```

## License

MIT
