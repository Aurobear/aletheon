"""Synthetic CSV sensor data summary statistics.

BUG: Median calculation forgets even-length case (should average two middle
elements). Variance uses n instead of n-1 for sample variance.
"""
from typing import List


def compute_stats(values: List[float]) -> dict:
    """Compute summary statistics for a list of sensor readings."""
    n = len(values)
    if n == 0:
        return {"count": 0}
    sorted_vals = sorted(values)
    # BUG: median is wrong for even-length lists
    median = sorted_vals[n // 2]
    mean = sum(values) / n
    # BUG: uses n (population variance) instead of n-1 (sample variance)
    variance = sum((v - mean) ** 2 for v in values) / n
    return {
        "count": n,
        "mean": mean,
        "median": median,
        "variance": variance,
        "std": variance ** 0.5,
        "min": sorted_vals[0],
        "max": sorted_vals[-1],
    }
