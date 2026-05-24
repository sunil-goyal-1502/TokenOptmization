#!/usr/bin/env bash
# Generate SWE-bench-style trace fixtures (no API keys).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
mkdir -p fixtures/swe_bench/traces
cargo run -p tokenopt-cli -- bench agent-loop --turns 18 --payload-bytes 10000 \
  --write-trace fixtures/swe_bench/traces/long_agent.json
cargo run -p tokenopt-cli -- bench agent-loop --turns 10 --payload-bytes 6000 \
  --write-trace fixtures/swe_bench/traces/transforms.json
cp fixtures/sample-trace.json fixtures/swe_bench/traces/core_compile.json
