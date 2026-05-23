# tokenopt (Python)

Production HTTP client for the TokenOpt context compiler. Use with LangGraph, CrewAI, AutoGen, or any Python agent harness.

```bash
pip install -e ./bindings/python
```

Start the server from the repo root:

```bash
cargo run -p tokenopt-server -- --bind 127.0.0.1:8787
```

```python
from tokenopt import TokenOptClient, CompileOptions

client = TokenOptClient()
messages = [{"role": "user", "content": "Fix src/main.rs"}]
result = client.compile(messages, CompileOptions(token_budget=80_000, session_id="s1"))
print(result.stats.tokens_saved, result.stats.reduction_percent)
```
