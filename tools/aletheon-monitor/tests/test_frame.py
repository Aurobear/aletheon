from src.frame import normalize, is_stable, changed


def test_normalize_strips_ansi_and_trailing_blanks():
    raw = "\x1b[1mhello\x1b[0m   \n\nworld  \n\n\n"
    assert normalize(raw) == "hello\n\nworld"


def test_is_stable_true_when_last_three_identical():
    assert is_stable(["a", "b", "x", "x", "x"], window=3) is True


def test_is_stable_false_when_still_changing():
    assert is_stable(["x", "x", "y"], window=3) is False


def test_is_stable_false_when_too_few():
    assert is_stable(["x", "x"], window=3) is False


def test_changed_ignores_ansi_and_trailing_ws():
    assert changed("\x1b[1mhi\x1b[0m  ", "hi") is False
    assert changed("hi", "bye") is True
