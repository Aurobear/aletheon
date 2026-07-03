# Quick Start

Get from zero to a running Aletheon agent in 5 minutes.

---

## Prerequisites

- Linux system (Ubuntu 22.04+ or Arch Linux recommended)
- Rust 1.85 or newer. The repository pins Rust 1.85 for reproducible local builds; newer stable toolchains, including Arch Linux's rolling Rust package, are verified separately in CI.
- At least 4 GB RAM, 2 GB disk space

---

## 1. Clone and Build

```bash
git clone https://github.com/Aurobear/aletheon.git
cd aletheon
cargo build --release
```

Build time depends on your machine; expect 3-8 minutes for a clean build.

---

## 2. Run the Test Suite

Verify the build is healthy:

```bash
cargo test --workspace
```

All 600+ tests should pass. If you see failures, check your Rust version (`rustc --version`).

---

## 3. Configure the Agent

Create a minimal configuration:

```bash
mkdir -p ~/.config/aletheon
cat > ~/.config/aletheon/config.toml << 'EOF'
[agent]
name = "my-first-agent"

[[providers]]
name = "openai"
url = "https://api.openai.com/v1"
api_key = "sk-..."
model = "gpt-4o"

[memory]
backend = "sqlite"
path = "~/.local/share/aletheon/memory.db"

[tools]
enabled = ["bash", "file", "http"]
EOF
```

See [configuration reference](../design/runtime/react-loop.md) for all options. The `[[providers]]` section supports any OpenAI-compatible or Anthropic-compatible endpoint.

---

## 4. Start the Daemon

```bash
./target/release/aletheond
```

The daemon starts a Unix socket listener at `/tmp/aletheon.sock`.

---

## 5. Interact with the Agent

In a separate terminal:

```bash
./target/release/interact "What files are in the current directory?"
```

The agent will reason, call the `bash` tool with `ls`, and return a structured response.

---

## Experience Self-Evolution

Self-Evolution is Aletheon's distinguishing feature. The agent reflects on completed tasks and adjusts its behavior over time.

Try giving it a repetitive task across multiple sessions, then ask it to reflect:

```bash
./target/release/interact "/reflect"
```

The agent produces a structured `ReflectionEntry` analyzing what worked, what failed, and what it learned. Over time, these reflections accumulate in episodic memory and drive behavior adjustments.

For a full walkthrough, see the [Self-Evolution Demo](../../examples/self-evolution-demo/README.md).

---

## Service Mode (systemd)

To run Aletheon as a persistent system service:

```ini
# /etc/systemd/system/aletheon.service
[Unit]
Description=Aletheon Agent Runtime
After=network.target

[Service]
Type=simple
User=aletheon
ExecStart=/usr/bin/aletheond
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable --now aletheon
sudo journalctl -u aletheon -f
```

---

## Next Steps

- [Core Concepts](concepts.md) -- understand SelfField, BrainCore, BodyRuntime
- [Self-Evolution](../architecture/self-evolution.md) -- how the agent learns and evolves
- [Linux Integration](../architecture/linux-integration.md) -- eBPF, systemd, FUSE details
- [Architecture Overview](../design/architecture-overview.md) -- full system architecture
- [Testing](../development/testing.md) -- how to run and write tests
