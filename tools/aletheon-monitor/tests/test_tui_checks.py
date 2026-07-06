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
