# NekoRCA

<div align="center">

**Neko 远程控制适配器网关**

(目前项目正在处于初始阶段，版本号在发布正式版本之前一直保持大版本为 0.x.x。本项目遵循 [语义化版本 2.0](https://semver.org/lang/zh-CN/) 规范)

[![Version](https://img.shields.io/badge/version-0.1.0-blue.svg)](https://github.com/Ringaire/NekoRCA)
[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-AGPL--3.0-orange.svg)](LICENSE)

NekoCLI 的远程控制网关 — 消息路由、平台适配、连接管理

[功能特性](#功能特性) • [快速开始](#快速开始) • [架构设计](#架构设计) • [配置](#配置)

</div>

---

## 功能特性

- 🌐 **WebSocket 服务** — 管理 NekoCLI 出站连接
- 🔌 **平台适配器** — Telegram / QQ / WeChat 消息收发
- 🧭 **消息路由** — 平台会话 ↔ CLI 实例绑定
- 🔑 **认证机制** — Token 验证，支持环境变量配置
- 📡 **Long Polling** — 无需公网即可接收消息
- 🔄 **结果回传** — CLI 处理结果自动转发回平台

## 快速开始

```bash
# 配置 TG bot token
export NEKO_TG_TOKEN=your_bot_token

# 启动
cargo run --release

# 或使用自定义端口和认证
nekorca --host 0.0.0.0 --port 8080 --auth-token my-secret
```

## 架构设计

```
┌──────────┐     ┌──────────┐     ┌──────────┐
│ Telegram  │     │   QQ     │     │  WeChat  │
│  Bot API  │     │  Bot API │     │  Bot API │
└─────┬────┘     └────┬─────┘     └────┬─────┘
      │               │                │
      └───────────────┬┴────────────────┘
                      │
              ┌───────┴────────┐
              │   NekoRCA      │
              │  (Rust 网关)    │
              │                │
              │  adapter/      │
              │  core/router   │
              │  cli/ws        │
              └───────┬────────┘
                      │ WebSocket
              ┌───────┴────────┐
              │   NekoCLI      │
              │  (AI Agent)    │
              └────────────────┘
```

## 配置

| 环境变量 | 说明 |
|---------|------|
| `NEKO_RCA_TOKEN` | RCA 认证 token |
| `NEKO_TG_TOKEN` | Telegram Bot token |

### CLI 参数

```bash
nekorca [OPTIONS]

Options:
      --host <HOST>         监听地址 [default: 0.0.0.0]
      --port <PORT>         监听端口 [default: 8080]
      --auth-token <TOKEN>  认证 token (也支持 NEKO_RCA_TOKEN 环境变量)
```

### NekoCLI 连接

```bash
# 在 NekoCLI TUI 中
/rca connect ws://服务器地址:8080/cli/ws

# 带 token
/rca connect ws://服务器地址:8080/cli/ws my-token
```

## 相关项目

| 项目 | 说明 |
|------|------|
| [NekoCLI](https://github.com/Ringaire/NekoCLI) | 终端 AI 编程助手 |
| [NekoApp](https://github.com/Ringaire/NekoApp) | 桌面图形界面 |

## 许可证

[AGPL-3.0](LICENSE)
