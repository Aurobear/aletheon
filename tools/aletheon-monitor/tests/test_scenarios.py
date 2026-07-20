from pathlib import Path

from src import scenarios


def test_preflight_records_real_provenance(monkeypatch, tmp_path):
    binary = tmp_path / "aletheon"
    binary.write_bytes(b"binary")
    values = iter([
        str(binary), "active", "active", "deadbeef",
        "Id=aletheon.service\nActiveState=active\nId=aletheon.socket\nActiveState=active",
    ])
    monkeypatch.setattr(scenarios, "_command", lambda *a, **k: next(values))
    monkeypatch.setattr(
        scenarios, "_socket_provenance",
        lambda: {"path": "/run/user/1000/aletheon/aletheon.sock", "uid": 1000,
                 "gid": 1000, "mode": "0600"},
    )
    result = scenarios.preflight(str(tmp_path))
    assert result["source_commit"] == "deadbeef"
    assert result["binary_sha256"]
    assert "ActiveState=active" in result["unit"]
