# Quick Start

Get from zero to a running Aletheon agent in 5 minutes.

---

## Prerequisites

- Linux system (Ubuntu 22.04+ or Arch Linux recommended)
- Rust toolchain 1.75.0+
- An LLM provider API key (Anthropic, OpenAI, DeepSeek, or local Ollama)

---

## 1. Clone and Build

```bash
git clone https://github.com/Aurobear/aletheon.git
cd aletheon
cargo build --release
```

Build time depends on your machine; expect 3-8 minutes for a clean build.

---

## 2. Configure the Agent

Create the config directory and a minimal configuration:

```bash
mkdir -p ~/.aletheon
cat > ~/.aletheon/config.toml << 'EOF'
[agent]
default_provider = "anthropic"
default_model = "claude-sonnet-4-20250514"

[[providers]]
name = "anthropic"
base_url = "https://api.anthropic.com"
transport = "anthropic"
EOF
```

Set your API key (pick one):

```bash
# Option A: environment variable
export ANTHROPIC_API_KEY="sk-ant-..."

# Option B: .env file (loaded automatically)
echo 'ANTHROPIC_API_KEY=sk-ant-...' > ~/.aletheon/.env
```

**Other providers** — add more `[[providers]]` entries. Any OpenAI-compatible API works:

```toml
[[providers]]
name = "deepseek"
base_url = "https://api.deepseek.com"
transport = "openai"
models = ["deepseek-chat"]

[[providers]]
name = "ollama"
base_url = "http://localhost:11434"
transport = "openai"
models = ["qwen3:8b"]
```

API keys resolve from config first, then env var `<NAME>_API_KEY` (e.g. `DEEPSEEK_API_KEY`).

See [`config/default.toml`](../../config/default.toml) for all options.

---

## 3. Quick Test (no daemon needed)

The fastest way to verify everything works:

```bash
./target/release/aletheon-exec --prompt "Say hello"
```

This runs a single prompt through the agent loop and prints the response. No daemon, no socket, no TUI — just a one-shot test.

---

## 4. System Service Mode (recommended for daily use)

Install as a system service with sudo:

```bash
# Copy binaries to system path
sudo cp target/release/aletheond /usr/local/bin/
sudo cp target/release/aletheon /usr/local/bin/
sudo cp target/release/aletheon-exec /usr/local/bin/

# Create systemd service
sudo tee /etc/systemd/system/aletheond.service > /dev/null << 'EOF'
[Unit]
Description=Aletheon Agent Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/aletheond
Restart=on-failure
RestartSec=5
EnvironmentFile=-/home/YOUR_USER/.aletheon/.env

[Install]
WantedBy=multi-user.target
EOF

# Edit EnvironmentFile to point to your actual user path, then:
sudo systemctl daemon-reload
sudo systemctl enable --now aletheond
sudo journalctl -u aletheond -f
```

The daemon creates its socket at `/run/aletheond/aletheond.sock` (default).

---

## 5. Interact via TUI

```bash
# Single message mode
aletheon -m "What files are in the current directory?"

# Interactive TUI mode
aletheon --tui
```

---

## Alternative: User Mode (no sudo)

For development/testing without system-wide install:

```bash
# Start daemon with user-writable socket
./target/release/aletheond --socket /tmp/aletheon/aletheon.sock &

# Connect TUI to that socket
./target/release/aletheon -m "hello" -s /tmp/aletheon/aletheon.sock
```

---

## Binary Reference

| Binary | Purpose | Example |
|--------|---------|---------|
| `aletheon-exec` | One-shot execution (no daemon) | `aletheon-exec --prompt "fix the bug"` |
| `aletheond` | Daemon (persistent, serves TUI) | `aletheond` (uses config defaults) |
| `aletheon` | TUI client (connects to daemon) | `aletheon -m "hello"` or `aletheon --tui` |

---

## Next Steps

- [Core Concepts](concepts.md) — understand SelfField, BrainCore, BodyRuntime
- [Self-Evolution](../architecture/self-evolution.md) — how the agent learns and evolves
- [Linux Integration](../architecture/linux-integration.md) — eBPF, systemd, FUSE details
- [Architecture Overview](../design/architecture-overview.md) — full system architecture
- [`config/default.toml`](../../config/default.toml) — full configuration reference
