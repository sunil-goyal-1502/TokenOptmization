#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TOKENOPT_URL="${TOKENOPT_URL:-http://127.0.0.1:8787}"
PROVIDER="${TOKENOPT_E2E_PROVIDER:-ollama}"

if ! curl -sf "$TOKENOPT_URL/health" >/dev/null 2>&1; then
  echo "Starting tokenopt-server on $TOKENOPT_URL ..."
  cargo run -p tokenopt-server -- --bind 127.0.0.1:8787 &
  SERVER_PID=$!
  trap 'kill $SERVER_PID 2>/dev/null || true' EXIT
  for _ in $(seq 1 40); do
    curl -sf "$TOKENOPT_URL/health" >/dev/null && break
    sleep 0.25
  done
fi

if [[ "$PROVIDER" == "ollama" ]]; then
  if ! curl -sf http://127.0.0.1:11434/api/tags >/dev/null 2>&1; then
    echo "Start Ollama: ollama serve"
    exit 1
  fi
  if ! ollama list | grep -q .; then
    echo "Pull a model: ollama pull tinyllama"
    exit 1
  fi
else
  if [[ -z "${OPENAI_API_KEY:-}" ]]; then
    echo "Set OPENAI_API_KEY or TOKENOPT_E2E_PROVIDER=ollama"
    exit 1
  fi
fi

pip install -q -r examples/e2e/requirements.txt
python3 examples/e2e/run_real_agent_e2e.py \
  --provider "$PROVIDER" \
  --turns "${TOKENOPT_E2E_TURNS:-6}" \
  --padding-kb "${TOKENOPT_E2E_PADDING_KB:-4}" \
  "$@"
