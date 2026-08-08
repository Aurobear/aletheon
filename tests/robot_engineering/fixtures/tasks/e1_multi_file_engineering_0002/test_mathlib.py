"""Tests for mathlib.

BUG: Only clamp has tests. lerp and normalize_angle are untested.
"""
import mathlib


def test_clamp():
    assert mathlib.clamp(5, 0, 10) == 5
    assert mathlib.clamp(-5, 0, 10) == 0
    assert mathlib.clamp(15, 0, 10) == 10
    assert mathlib.clamp(0, 0, 10) == 0
    assert mathlib.clamp(10, 0, 10) == 10


# BUG: lerp and normalize_angle have no tests


def run_all():
    test_clamp()
    print("OK: all tests passed")


if __name__ == "__main__":
    run_all()
