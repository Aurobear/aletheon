"""Synthetic robot log anomaly detector — FIXED.

Uses proper severity level comparison instead of string ordering.
"""
from typing import List, Dict

SEVERITY_ORDER = {"DEBUG": 0, "INFO": 1, "WARN": 2, "ERROR": 3, "FATAL": 4}


def detect_anomalies(log_lines: List[str], min_severity: str = "WARN") -> List[Dict]:
    min_level = SEVERITY_ORDER.get(min_severity, 0)
    anomalies = []
    for line in log_lines:
        for sev in ["FATAL", "ERROR", "WARN", "INFO", "DEBUG"]:
            if line.startswith(sev):
                if SEVERITY_ORDER.get(sev, 0) >= min_level:
                    anomalies.append({"line": line, "severity": sev})
                break
    return anomalies
