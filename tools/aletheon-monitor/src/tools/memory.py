"""aletheon_memory — query agent memory system."""

from ..client import AletheonClient


async def memory(
    client: AletheonClient,
    query: str,
    memory_type: str = "all",
    limit: int = 10,
) -> dict:
    """Search the agent's memory store.

    Args:
        client: Connected AletheonClient.
        query: Search query string.
        memory_type: "core", "recall", "facts", or "all" (default).
        limit: Max results to return.
    """
    params = {
        "query": query,
        "memory_type": memory_type,
        "limit": limit,
    }

    resp = await client.rpc("session.memory", params)

    if "error" in resp:
        return {"error": resp["error"], "entries": []}

    result = resp.get("result", resp)
    entries = result.get("entries", result.get("results", result.get("memory", [])))

    return {
        "query": query,
        "memory_type": memory_type,
        "entries": entries,
        "count": len(entries),
    }
