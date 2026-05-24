# Security

## Scan summary (pre-merge)

| Check | Result |
|-------|--------|
| Hardcoded API keys / secrets in repo | **None found** |
| `.env` committed | **No** (gitignored) |
| `sk-...` in docs | Placeholders only (`sk-...`) |
| E2E reports / cold store | **Gitignored** |
| `target/`, `node_modules`, `*.node` | **Not tracked** |

## Threat model

`tokenopt-server` is intended as a **local sidecar** (default bind `127.0.0.1:8787`), not a public multi-tenant service.

| Risk | Mitigation |
|------|------------|
| **No authentication** | Bind to localhost; do not expose without a reverse proxy + auth |
| **Permissive CORS** | Acceptable for local dev; restrict in production deployments |
| **Cold-store path traversal** | `session_id` / `key` sanitized in `FileColdStore` |
| **Config path reads** (`guideline_bank_path`, `fold_policy_path`) | Reject `..`; operators should only pass trusted paths |
| **E2E tools** | `read_file` / `list_dir` constrained to repo workspace |
| **LLM API keys** | Read from env (`OPENAI_API_KEY`); never logged or stored in repo |

## Reporting

Open a private security advisory on GitHub for sensitive issues.
