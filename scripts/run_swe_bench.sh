#!/usr/bin/env bash
# SWE-bench-lite: Rust integration test (no API keys).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
if [[ ! -f fixtures/swe_bench/traces/long_agent.json ]]; then
  ./scripts/generate_swe_traces.sh
fi
cargo test -p tokenopt-core --test swe_bench_harness -- --nocapture
