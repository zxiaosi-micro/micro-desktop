# micro-desktop · 运维 PC（Tauri）

Tauri 2 + Rust 独立工具链仓（ADR-17：Rust 工具链隔离）。运维 PC 面向运维工程师，**前端复用 micro-web/apps/admin 构建产物**，Rust 三模块：

| 模块 | 职责 |
|---|---|
| diag | 串口/CAN/BLE 诊断（JBD 协议帧，仅本地运行） |
| flash | XMODEM-1K 固件刷写（断点续传 + sha256 缓存） |
| transfer | 文件传输（CSV/XLSX/zip） |

> S9 落地。产物 ~10MB（对比 Electron 100MB+）；capabilities 白名单；Rust 库选型 serialport / btleplug。

## 常用命令（脚手架落地后可用）

```bash
make check    # cargo check
make clippy   # cargo clippy -- -D warnings
make fmt      # cargo fmt
make build    # cargo build --release（src-tauri）
```

## 提交约定（S0-03）

中文 conventional commits + 任务号（`feat(flash): S9-02 断点续传`）；PR 走 DoD 自查模板。
