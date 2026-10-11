# icons 占位说明

`tauri build` 要求本目录存在 `icon.ico`（tauri.conf.json bundle.icon 引用）。
本仓不入库二进制图标——构建前执行一次：

```bash
# 任意 1024x1024 PNG 源图（如平台 logo）
pnpm tauri icon path/to/logo.png
# 产出 icons/icon.ico 与全尺寸 png/ico 副本
```

未生成图标前，`tauri build` 报错 missing icon 属预期（Windows 实机构建验证步骤见 README）。
