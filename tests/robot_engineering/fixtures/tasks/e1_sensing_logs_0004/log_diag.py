"""Synthetic robot log anomaly detector.

BUG: Classifies log entries by severity but uses string prefix matching
instead of proper severity comparison, so 'WARN' > 'ERROR' in ASCII.
"""
from typing import List, Dict

# Severity levels (higher = more severe)
SEVERITY_ORDER = {"DEBUG": 0, "INFO": 1, "WARN": 2, "ERROR": 3, "FATAL": 4}


def detect_anomalies(log_lines: List[str], min_severity: str = "WARN") -> List[Dict]:
    """Find log lines at or above min_severity."""
    anomalies = []
    for line in log_lines:
        # BUG: uses string prefix match instead of severity level
        for sev in ["FATAL", "ERROR", "WARN", "INFO", "DEBUG"]:
            if line.startswith(sev):
                # BUG: string comparison — "WARN" > "ERROR" in ASCII
                if sev > min_severity:
                    anomalies.append({"line": line, "severity": sev})
                break
    return anomalies
