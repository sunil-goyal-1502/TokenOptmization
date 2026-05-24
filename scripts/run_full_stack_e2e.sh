#!/usr/bin/env bash
# Full-stack E2E: server + compress sidecar + HTTP API + real LLM agent + SWE harness.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ -f "$ROOT/.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$ROOT/.env"
  set +a
fi

PROVIDER="${TOKENOPT_E2E_PROVIDER:-ollama}"
export TOKENOPT_E2E_PROVIDER="$PROVIDER"
SERVER_PID=""
SIDECAR_PID=""

cleanup() {
  [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null || true
  [[ -n "$SIDECAR_PID" ]] && kill "$SIDECAR_PID" 2>/dev/null || true
}
trap cleanup EXIT

echo "=== 1. Build ==="
cargo build --release -p tokenopt-server -p tokenopt-cli -p tokenopt-core

echo "=== 2. Start tokenopt-server :8787 ==="
./target/release/tokenopt-server --bind 127.0.0.1:8787 &
SERVER_PID=$!
for _ in $(seq 1 40); do
  curl -sf http://127.0.0.1:8787/health >/dev/null && break
  sleep 0.25
done
curl -sf http://127.0.0.1:8787/health
echo ""

echo "=== 3. Start compress sidecar :8790 ==="
python3 examples/compress_sidecar.py &
SIDECAR_PID=$!
sleep 1
curl -sf -X POST http://127.0.0.1:8790/compress \
  -H 'Content-Type: application/json' \
  -d '{"blocks":[{"id":"b1","kind":"ToolResult","content":"'$(python3 -c "print('x'*500)")'"}]}' | head -c 200
echo ""

echo "=== 4. Rust SWE-bench harness (compile on traces) ==="
cargo test -p tokenopt-core --test swe_bench_harness --release

echo "=== 5. HTTP API smoke (research pipeline options) ==="
python3 <<'PY'
import json
import sys
import urllib.request

ROOT = __import__("pathlib").Path(".").resolve()
trace = json.loads((ROOT / "fixtures/swe_bench/traces/long_agent.json").read_text())
messages = trace.get("messages", trace)

opts = {
    "session_id": "full-stack-e2e",
    "token_budget": 128000,
    "keep_recent_tool_results": 2,
    "run_sufficiency_check": False,
    "soft_sufficiency": True,
    "transforms": {
        "guideline_bank": True,
        "fold_policy": True,
        "summarization": True,
        "external_compress": True,
        "memory_prune": True,
        "bacm": False,
    },
    "guideline_bank_path": str(ROOT / "fixtures/guidelines/default.json"),
    "fold_policy_path": str(ROOT / "fixtures/fold_policies/default.json"),
    "external_compress_url": "http://127.0.0.1:8790/compress",
    "routing_hints": True,
    "turn_index": 12,
}

def post(path, body):
    req = urllib.request.Request(
        f"http://127.0.0.1:8787{path}",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read())

cmp = post("/v1/compare", {"messages": messages, "options": opts})
print(f"compare: reduction={cmp['reduction_percent']:.1f}% cold_refs={cmp['cold_refs_count']}")
assert cmp["reduction_percent"] > 10, "expected >10% on long trace"

compiled = post("/v1/compile", {"messages": messages, "options": opts})
names = compiled["stats"]["transforms_applied"]
print(f"compile: transforms={names}")
assert "consumed_result_mask" in str(names) or "rule_summarize" in str(names)

rh = compiled.get("routing_hint")
print(f"routing_hint: {rh}")

fold = post("/v1/fold/collapse", {
    "messages": messages + [{"role": "fold", "name": "branch-a", "content": "done exploring"}]
})
print(f"fold_collapse: records={len(fold.get('fold_records', []))}")

print("HTTP research pipeline: OK")
PY

echo "=== 6. Real LLM agent (baseline compile options) ==="
pip install -q -r examples/e2e/requirements.txt
pip install -q -e bindings/python 2>/dev/null || true
python3 examples/e2e/run_real_agent_e2e.py --provider "$PROVIDER" --turns 4 --padding-kb 4

echo "=== 7. Real LLM agent (research + sidecar) ==="
python3 examples/e2e/run_real_agent_e2e.py --provider "$PROVIDER" --turns 4 --padding-kb 6 --research

echo "=== 8. Quality A/B (2 turns per task) ==="
python3 examples/e2e/run_quality_ab.py --provider "$PROVIDER" --turns-per-task 2

echo "=== 9. Native Python (optional) ==="
if command -v maturin >/dev/null 2>&1; then
  pip install -q maturin
  cd bindings/python && maturin develop --release 2>/dev/null && cd "$ROOT"
  python3 -c "
from tokenopt.native import native_available, compile_native
print('native_available', native_available())
if native_available():
    import json
    from pathlib import Path
    t=json.loads(Path('fixtures/sample-trace.json').read_text())
    m=t['messages']
    r=compile_native(m, __import__('tokenopt').CompileOptions(session_id='n', run_sufficiency_check=False))
    print('native reduction', r['stats']['reduction_percent'])
"
else
  echo "SKIP maturin not installed"
fi

echo ""
echo "=== FULL STACK E2E: PASSED ==="
