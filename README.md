# RingRCA — 远程控制适配器

Remote Control Adapter — ring 生态的远程控制网关，连接 RingApp（桌面/移动端）与 RingCLI 工作进程。

## 架构

```
  RingApp (Tauri)                Platform APIs (TG/QQ/WeChat)
       │                                │
       │ ws /app/ws            webhook POST /api/v1/message/{platform}
       ▼                                ▼
    ┌──────────────────────────────────────┐
    │            RingRCA Server            │
    │  ┌─────────┐  ┌───────────────────┐  │
    │  │ Router  │  │ Dispatcher        │  │
    │  │ routes  │  │ dispatches results│  │
    │  └────┬────┘  └───────┬───────────┘  │
    └───────┼───────────────┼──────────────┘
            │ ws /cli/ws    │
            ▼               ▼
    ┌──────────────┐  platform adapters
    │   RingCLI    │  (tg.rs / qq.rs / wechat.rs)
    │   worker(s)  │
    └──────────────┘
```

## 端点

| 端点 | 方法 | 用途 |
|------|------|------|
| `/health` | GET | 健康检查 |
| `/cli/ws` | WS | RingCLI worker 连接（接收任务、返回结果） |
| `/app/ws` | WS | RingApp 连接（派发任务、接收结果） |
| `/api/v1/message/{platform}` | POST | 平台 webhook（Telegram/QQ/WeChat → CLI） |

## 运行

```bash
# 启动（默认 0.0.0.0:8080）
cargo run --release

# 带 auth token + Telegram bot
RING_RCA_TOKEN=my-secret RING_TG_TOKEN=123:abc ringrca

# RingApp 连接地址
ws://host:8080/app/ws
```

## 协议

所有 WS 消息使用统一的 `Envelope` 格式：

```json
{
  "id": "<uuid>",
  "type": "register|register_ack|assign_task|task_result|heartbeat|...",
  "payload": { ... },
  "timestamp": 1700000000000,
  "direction": "upstream|downstream"
}
```

### App 流程（/app/ws）

1. App 连接 → 发送 `register`（含 auth_token）
2. Server 返回 `register_ack`
3. App 发送 `assign_task`（含 task_id、conversation_id、message.text）
4. Server 路由到可用的 CLI worker
5. CLI 处理完毕 → 发送 `task_result` 给 server
6. Server 将 `task_result` 转发回 app

## 许可证

[AGPL-3.0](LICENSE)
