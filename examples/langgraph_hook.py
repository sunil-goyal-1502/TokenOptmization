"""Example: LangGraph-style node that compiles context before the LLM step."""

from tokenopt import CompileOptions, TokenOptClient


def compile_state_messages(state: dict, session_id: str) -> dict:
    client = TokenOptClient()
    try:
        compiled = client.before_model(
            state["messages"],
            session_id=session_id,
            options=CompileOptions(token_budget=100_000, soft_sufficiency=True),
        )
        return {**state, "messages": [m.model_dump(exclude_none=True) for m in compiled]}
    finally:
        client.close()
