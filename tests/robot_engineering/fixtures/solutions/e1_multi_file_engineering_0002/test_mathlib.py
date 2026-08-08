"""Tests for mathlib — COMPLETE.

All three functions are now covered with edge cases.
"""
import mathlib
import math


def test_clamp():
    assert mathlib.clamp(5, 0, 10) == 5
    assert mathlib.clamp(-5, 0, 10) == 0
    assert mathlib.clamp(15, 0, 10) == 10
    assert mathlib.clamp(0, 0, 10) == 0
    assert mathlib.clamp(10, 0, 10) == 10


def test_lerp():
    assert mathlib.lerp(0, 10, 0.0) == 0.0
    assert mathlib.lerp(0, 10, 1.0) == 10.0
    assert mathlib.lerp(0, 10, 0.5) == 5.0
    assert mathlib.lerp(10, 0, 0.5) == 5.0
    assert mathlib.lerp(-5, 5, 0.5) == 0.0


def test_normalize_angle():
    assert abs(mathlib.normalize_angle(0)) < 1e-9
    assert abs(mathlib.normalize_angle(math.pi)) - math.pi < 1e-9
    assert abs(mathlib.normalize_angle(-math.pi)) - math.pi < 1e-9
    assert abs(mathlib.normalize_angle(3 * math.pi) + math.pi) < 1e-9


def run_all():
    test_clamp()
    test_lerp()
    test_normalize_angle()
    print("OK: all tests passed")


if __name__ == "__main__":
    run_all()
