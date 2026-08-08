"""Synthetic CSV sensor data summary statistics — FIXED.

Correct median for even-length lists and uses n-1 for sample variance.
"""
from typing import List


def compute_stats(values: List[float]) -> dict:
    n = len(values)
    if n == 0:
        return {"count": 0}
    sorted_vals = sorted(values)
    if n % 2 == 0:
        median = (sorted_vals[n // 2 - 1] + sorted_vals[n // 2]) / 2.0
    else:
        median = sorted_vals[n // 2]
    mean = sum(values) / n
    variance = sum((v - mean) ** 2 for v in values) / (n - 1) if n > 1 else 0.0
    return {
        "count": n,
        "mean": mean,
        "median": median,
        "variance": variance,
        "std": variance ** 0.5,
        "min": sorted_vals[0],
        "max": sorted_vals[-1],
    }
