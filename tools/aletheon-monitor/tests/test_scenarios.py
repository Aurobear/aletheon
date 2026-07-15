from pathlib import Path

from src import scenarios


def test_preflight_records_real_provenance(monkeypatch, tmp_path):
    binary = tmp_path / "aletheon"
    binary.write_bytes(b"binary")
    values = iter([str(binary), "deadbeef", "ActiveState=active"])
    monkeypatch.setattr(scenarios, "_command", lambda *a, **k: next(values))
    result = scenarios.preflight(str(tmp_path))
    assert result["source_commit"] == "deadbeef"
    assert result["binary_sha256"]
    assert "ActiveState=active" in result["unit"]
