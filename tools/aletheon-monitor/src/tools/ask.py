"""aletheon_ask — send a question to the running agent."""

from ..client import AletheonClient


async def ask(client: AletheonClient, question: str) -> dict:
    """Send a question to the running agent for introspection.

    Uses the agent's own LLM via session.ask. Rate-limited by the daemon.

    Args:
        client: Connected AletheonClient.
        question: Question to ask the agent.
    """
    resp = await client.rpc("session.ask", {"question": question})

    if "error" in resp:
        return {"error": resp["error"], "question": question}

    result = resp.get("result", resp)
    return {
        "question": question,
        "response": result.get("response", result.get("answer", str(result))),
    }
