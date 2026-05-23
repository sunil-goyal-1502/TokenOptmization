#!/usr/bin/env bash
# Quality A/B: baseline vs TokenOpt on planned tool tasks (real LLM).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ -f "$ROOT/.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$ROOT/.env"
  set +a
fi

if ! curl -sf "http://127.0.0.1:8787/health" >/dev/null 2>&1; then
  echo "Starting tokenopt-server on :8787..."
  cargo run -p tokenopt-server -- --bind 127.0.0.1:8787 &
  SERVER_PID=$!
  trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT
  for _ in $(seq 1 40); do
    curl -sf "http://127.0.0.1:8787/health" >/dev/null && break
    sleep 0.25
  done
fi

pip install -q -r examples/e2e/requirements.txt
pip install -q -e bindings/python 2>/dev/null || pip install -q httpx pydantic

python3 examples/e2e/run_quality_ab.py "$@"
