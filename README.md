# micro-desktop · 运维 PC（Tauri 2）

Tauri 2 + Rust 独立工具链仓（ADR-17：Rust 工具链隔离）。运维 PC 面向运维工程师，**前端复用 micro-web/apps/admin 构建产物**（tauri.conf.json `frontendDist` 指向 `../../micro-web/apps/admin/dist`），Rust 三模块：

- **diag**：串口/CAN/BLE 诊断（JBD 协议帧，仅本地运行不上云）
- **flash**：XMODEM-1K 固件刷写（断点续传 + sha256 缓存）
- **transfer**：文件传输（CSV/XLSX/zip）

S9 落地（S9-05）。产物 ~10MB（对比 Electron 100MB+）；capabilities 白名单；Rust 库选型 serialport / btleplug。
（提交约定：中文 conventional commits + 任务号，如 `feat(flash): S9-05 断点续传`）

## 目录结构

```
micro-desktop/
├── package.json            # @tauri-apps/cli 入口（tauri dev / build）
└── src-tauri/
    ├── Cargo.toml          # tauri 2 + serialport + btleplug + sha2 + csv/xlsx/zip
    ├── tauri.conf.json     # frontendDist → micro-web/apps/admin/dist（复用产物）
    ├── capabilities/default.json   # 白名单：core + dialog + fs scoped($DOWNLOAD/$TEMP)
    └── src/
        ├── main.rs / lib.rs    # command 注册
        ├── diag.rs             # JBD 帧解析（寄存器 0x03 基本信息）+ SLCAN/BLE 骨架
        ├── flash.rs            # XMODEM-1K 状态机 + {固件}.flash-progress 断点续传 + sha256 缓存
        └── transfer.rs         # export_csv（UTF-8 BOM）/ export_xlsx / pack_logs(zip)
```

## 构建前置（Windows 实机）

1. **Rust 工具链**（S9 阶段验收前置，`rustup` 预装验证放最前）：
   ```bash
   rustc --version        # ≥1.77（2021 edition）
   cargo --version
   ```
2. Node/pnpm（与 micro-web 同环境）；`pnpm install`（装 @tauri-apps/cli）
3. **生成图标**（不入库二进制）：`pnpm tauri icon <1024x1024 logo.png>` → `src-tauri/icons/icon.ico`
4. **前端产物**：`cd ../micro-web && pnpm --filter admin build`（tauri.conf.json 的 beforeBuildCommand 也会自动跑）

## 常用命令

```bash
make check        # cargo check
make clippy       # cargo clippy -D warnings
make build        # cargo build --release（Rust 侧单编）
pnpm tauri dev    # 开发联调（devUrl → admin vite :5173）
pnpm tauri build  # 产物 bundle（NSIS 安装包）
```

## 能力边界（01 §8.6 / 02 §12.4）

- **仅本地**：diag 采集数据不上云；导出走 transfer 本地落盘（$DOWNLOAD/$TEMP scoped）
- **非 Tauri 环境拒绝**：admin `/diag` 页探测 `window.__TAURI`，纯浏览器打开时占位拒绝 invoke
- **实机校准位**：JBD 固件字段精度、SLCAN 盒波特率、JBD-over-BLE 透传 UUID 按 ADR-19（协议标准+模拟器口径）标注于代码注释，实机到位后补兼容记录
- 断点续传进度文件 `{固件}.flash-progress`：烧录成功后由前端调 `flash_clear_progress` 清理
