# RingRCA

<div align="center">

**Ring Remote Control Adapter — Message Gateway**

(Project in early stage. Version stays 0.x.x until first stable release. [Semantic Versioning 2.0](https://semver.org/))

[![Version](https://img.shields.io/badge/version-0.1.0-blue.svg)](https://github.com/Ringaire/RingRCA)
[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-AGPL--3.0-orange.svg)](LICENSE)

Remote control gateway for RingCLI — message routing, platform adapters, connection management

[Features](#features) • [Quick Start](#quick-start) • [Architecture](#architecture) • [Configuration](#configuration)

</div>

---

## Features

- 🌐 **WebSocket server** — manages RingCLI outbound connections
- 🔌 **Platform adapters** — Telegram / QQ / WeChat message relay
- 🧭 **Message routing** — platform conversation ↔ CLI session binding
- 🔑 **Authentication** — token-based, supports env vars
- 📡 **Long polling** — no public URL needed for incoming messages
- 🔄 **Result dispatch** — auto forwards CLI responses back to platform

## Quick Start

```bash
export RING_TG_TOKEN=your_bot_token
cargo run --release

# Custom port and auth
ringrca --host 0.0.0.0 --port 8080 --auth-token my-secret
```

## Architecture

```
  Platform APIs (TG/QQ/WeChat)
           │
    ┌──────┴────────┐
    │   RingRCA     │
    │  adapter/     │
    │  core/router  │
    │  cli/ws       │
    └──────┬────────┘
           │ WebSocket
    ┌──────┴────────┐
    │   RingCLI     │
    └───────────────┘
```

## Configuration

| Env var | Description |
|---------|-------------|
| `RING_RCA_TOKEN` | RCA auth token |
| `RING_TG_TOKEN` | Telegram Bot token |

### CLI

```bash
ringrca [OPTIONS]

Options:
      --host <HOST>         Listen address [default: 0.0.0.0]
      --port <PORT>         Listen port [default: 8080]
      --auth-token <TOKEN>  Auth token (also reads RING_RCA_TOKEN)
```

### Connect from RingCLI

```bash
# In RingCLI TUI:
/rca connect ws://host:8080/cli/ws
# With token:
/rca connect ws://host:8080/cli/ws my-token
```

## Related Projects

| Project | Description |
|---------|-------------|
| [RingCLI](https://github.com/Ringaire/RingCLI) | Terminal AI coding assistant |
| [RingApp](https://github.com/Ringaire/RingApp) | Desktop GUI |

## License

[AGPL-3.0](LICENSE)
