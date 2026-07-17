from src.client import AletheonClient, default_socket_path


def test_default_socket_is_private_to_effective_user(monkeypatch):
    monkeypatch.delenv("ALETHEON_SOCKET", raising=False)
    monkeypatch.delenv("XDG_RUNTIME_DIR", raising=False)
    monkeypatch.setattr("src.client.os.geteuid", lambda: 1234)
    assert default_socket_path() == "/run/user/1234/aletheon/aletheon.sock"
    assert AletheonClient().socket_path == "/run/user/1234/aletheon/aletheon.sock"


def test_xdg_runtime_directory_selects_private_socket(monkeypatch):
    monkeypatch.delenv("ALETHEON_SOCKET", raising=False)
    monkeypatch.setenv("XDG_RUNTIME_DIR", "/run/user/42")
    assert AletheonClient().socket_path == "/run/user/42/aletheon/aletheon.sock"
