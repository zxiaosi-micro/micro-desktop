//! micro-desktop 库入口：注册 diag / flash / transfer 三模块 tauri command（S9-05）。
//!
//! capabilities 白名单见 src-tauri/capabilities/default.json——仅 core/dialog/fs(scoped)；
//! 本地能力全部经 invoke 自定义 command，非 Tauri 环境（纯浏览器打开 admin）无法触达，
//! /diag 页以 window.__TAURI 探测做占位拒绝（01 §8.6）。

pub mod diag;
pub mod flash;
pub mod transfer;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            // diag：串口（JBD 帧）/ CAN（SLCAN 骨架）/ BLE（btleplug 骨架）
            diag::list_serial_ports,
            diag::diag_serial_basic_info,
            diag::diag_slcan_read,
            diag::diag_ble_scan,
            diag::diag_ble_basic_info,
            // flash：XMODEM-1K + 断点续传 + sha256 缓存
            flash::firmware_sha256,
            flash::flash_xmodem_1k,
            flash::flash_clear_progress,
            // transfer：CSV / XLSX / zip
            transfer::export_csv,
            transfer::export_xlsx,
            transfer::pack_logs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running micro-desktop");
}
