from src.tui_checks import check_dup_render


def test_dup_render_detects_repeated_block():
    frame = "\n".join([
        "line one",
        "reflection A",
        "reflection B",
        "reflection C",
        "some other output",
        "reflection A",
        "reflection B",
        "reflection C",
        "tail",
    ])
    findings = check_dup_render(frame, min_block=3)
    assert len(findings) == 1
    assert findings[0]["kind"] == "dup_render"
    assert "reflection A" in findings[0]["evidence"]


def test_dup_render_ignores_short_or_blank_repeats():
    frame = "\n".join(["a", "", "a", "", "b", "", "b"])
    assert check_dup_render(frame, min_block=3) == []


import os
from src.tui_checks import (
    check_raw_markdown,
    check_double_reflection,
    check_unknown_skill_path,
    check_permission_denied,
    run_checks,
)

_FIXTURE = os.path.join(
    os.path.dirname(__file__), "fixtures", "real_session_dup.txt"
)


def test_raw_markdown_detects_table_pipes():
    assert check_raw_markdown("| # | 问题 |\n|---|------|")[0]["kind"] == "raw_markdown"
    assert check_raw_markdown("normal text") == []


def test_double_reflection_prefix():
    assert check_double_reflection("Reflection: Reflection: 20 calls")[0]["kind"] == "double_reflection"
    assert check_double_reflection("Reflection: 20 calls") == []


def test_unknown_skill_path():
    assert check_unknown_skill_path("未知技能: /home/aurobear/x")[0]["kind"] == "unknown_skill_path"
    assert check_unknown_skill_path("未知技能: foo") == []


def test_permission_denied():
    assert check_permission_denied("touch: Permission denied")[0]["kind"] == "permission_denied"
    assert check_permission_denied("all good") == []


def test_run_checks_on_real_session_fixture():
    """The real captured session must trip the major render/behaviour checks."""
    with open(_FIXTURE, encoding="utf-8", errors="replace") as f:
        frame = f.read()
    kinds = {c["kind"] for c in run_checks(frame)}
    assert "dup_render" in kinds
    assert "raw_markdown" in kinds
    assert "double_reflection" in kinds
    assert "permission_denied" in kinds
