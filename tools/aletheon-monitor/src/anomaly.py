"""Anomaly detection rules for aletheon_analyze.

Each rule is a pure function that takes the merged analyze data and returns
``None`` (no anomaly) or an anomaly dict with ``type``, ``severity``, and
``detail`` fields.
"""

from typing import Optional


def check_restart_loop(perf: dict) -> Optional[dict]:
    """Detect rapid crash-restart cycles (systemd restart_counter)."""
    # restart_count is embedded in the health response; analyze merges it
    restart_count = perf.get("restart_count", 0)
    if restart_count > 5:
        return {
            "type": "restart_loop",
            "severity": "CRITICAL",
            "detail": f"Restart count {restart_count} > 5 — daemon in crash loop",
        }
    return None


def check_tool_error_rate(perf: dict) -> list[dict]:
    """Per-tool error rate > 10% warns, > 25% is critical."""
    anomalies = []
    by_tool = (
        perf.get("tool_calls", {})
        .get("by_tool", {})
    )
    for tool_name, stats in by_tool.items():
        total = stats.get("total", 0)
        errors = stats.get("errors", 0)
        if total < 5:
            continue  # Not enough data
        rate = errors / total
        if rate > 0.25:
            anomalies.append({
                "type": "tool_error_rate",
                "severity": "CRITICAL",
                "detail": f"{tool_name}: {rate:.0%} error rate ({errors}/{total})",
            })
        elif rate > 0.10:
            anomalies.append({
                "type": "tool_error_rate",
                "severity": "WARN",
                "detail": f"{tool_name}: {rate:.0%} error rate ({errors}/{total})",
            })
    return anomalies


def check_llm_error_rate(perf: dict) -> Optional[dict]:
    """LLM call error rate > 5% warns."""
    llm = perf.get("llm_calls", {})
    total = llm.get("total", 0)
    errors = llm.get("errors", 0)
    if total < 10:
        return None
    rate = errors / total
    if rate > 0.05:
        return {
            "type": "llm_error_rate",
            "severity": "WARN",
            "detail": f"LLM error rate {rate:.0%} ({errors}/{total})",
        }
    return None


def check_context_overflow(journal: list[dict]) -> Optional[dict]:
    """> 3 compaction events in the last 10 journal entries."""
    compacted = sum(
        1 for e in journal
        if e.get("event_type") in ("compacted", "Compacted")
    )
    if compacted > 3:
        return {
            "type": "context_overflow",
            "severity": "WARN",
            "detail": f"{compacted} compactions in recent journal — context pressure high",
        }
    return None


def check_socket(snapshot: dict) -> Optional[dict]:
    """Socket file missing or unwritable."""
    sock = snapshot.get("socket", {})
    if not sock.get("exists", True):
        return {
            "type": "socket_missing",
            "severity": "CRITICAL",
            "detail": f"Socket not found at {sock.get('path', 'unknown')}",
        }
    if not sock.get("writable", True):
        return {
            "type": "socket_missing",
            "severity": "CRITICAL",
            "detail": f"Socket not writable at {sock.get('path', 'unknown')}",
        }
    return None


def check_provider(snapshot: dict) -> Optional[dict]:
    """LLM provider health check failed."""
    providers = snapshot.get("providers", [])
    for p in providers:
        if not p.get("healthy", True):
            return {
                "type": "provider_unreachable",
                "severity": "CRITICAL",
                "detail": f"Provider '{p.get('name', 'unknown')}' unreachable",
            }
    return None


def check_memory_growth(_perf: dict) -> Optional[dict]:
    """Memory growth anomaly. Placeholder — requires baseline tracking."""
    # Full implementation requires storing baselines across calls.
    # For P3 we report gross memory figures; Claude can track deltas.
    return None


# Ordered list of all rules to run in analyze()
ALL_RULES = [
    ("restart_loop", check_restart_loop),
    ("tool_error_rate", check_tool_error_rate),
    ("llm_error_rate", check_llm_error_rate),
    ("context_overflow", check_context_overflow),
    ("socket_missing", check_socket),
    ("provider_unreachable", check_provider),
    ("memory_growth", check_memory_growth),
]


def run_all(perf: dict, snapshot: dict, journal: list[dict]) -> list[dict]:
    """Run all anomaly detection rules against the merged analyze data.

    Returns:
        List of anomaly dicts, empty if all clear.
    """
    anomalies = []
    for _name, rule in ALL_RULES:
        if rule is check_restart_loop or rule is check_llm_error_rate or rule is check_memory_growth:
            result = rule(perf)
            if result:
                if isinstance(result, list):
                    anomalies.extend(result)
                elif result:
                    anomalies.append(result)
        elif rule is check_tool_error_rate:
            anomalies.extend(rule(perf))
        elif rule is check_context_overflow:
            result = rule(journal)
            if result:
                anomalies.append(result)
        elif rule is check_socket or rule is check_provider:
            result = rule(snapshot)
            if result:
                anomalies.append(result)
    return anomalies
