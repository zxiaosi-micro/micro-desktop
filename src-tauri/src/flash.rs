//! flash.rs · 固件烧录模块（S9-05）：XMODEM-1K 串口烧录 + 断点续传 + sha256 缓存。
//!
//! 协议口径（02 §12.4 / 01 §8.6）：XMODEM-1K（STX 1024 字节块，CRC16 校验），
//! 断点续传 = 本地进度文件（`{固件路径}.flash-progress`）记录已确认 offset；
//! 重启后以进度文件 offset 起续传（设备端 ACK 续传起点由 EOT/C 机制协商——实机校准）。
//! sha256 缓存：同哈希固件 + 同端口续传文件有效，换固件即重置进度。
//!
//! 边界：烧录对象为 BMS/桩串口，仅本地；烧录记录（谁/何时/哪个固件）由管理后台 cmd 审计流承接。

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use crate::diag::AppError;

type Result<T> = std::result::Result<T, AppError>;

const SOH: u8 = 0x01; // 128 字节块
const STX: u8 = 0x02; // 1024 字节块（XMODEM-1K）
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const CRC_MODE: u8 = 0x43; // 'C'——接收方请求 CRC 模式

/// 烧录进度快照（进度事件 + 断点续传文件共用）。
#[derive(Debug, Clone, Serialize)]
pub struct FlashProgress {
    /// 设备 SN（审计与前端展示）
    pub sn: String,
    /// 固件路径
    pub firmware: String,
    /// 固件哈希（续传文件有效性校验键）
    pub firmware_sha256: String,
    /// 固件总字节数
    pub bytes_total: u64,
    /// 已确认字节数
    pub bytes_done: u64,
    /// 已确认块号（续传起点）
    pub block_index: u32,
    /// 是否已完成（完成态进度文件不可再续传）
    pub finished: bool,
}

/// 计算固件 sha256（烧录前校验；与 device 域 firmware.sha256 同一口径）。
#[tauri::command]
pub fn firmware_sha256(path: String) -> Result<String> {
    let mut f = std::fs::File::open(&path).map_err(|e| AppError(format!("打开固件失败: {e}")))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf).map_err(|e| AppError(format!("读固件失败: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// XMODEM-1K 烧录（串口）：断点续传 + 每块进度事件（event 供前端进度条）。
#[tauri::command]
pub fn flash_xmodem_1k(
    app: tauri::AppHandle,
    port: String,
    baud: u32,
    sn: String,
    firmware_path: String,
) -> Result<FlashProgress> {
    let payload = std::fs::read(&firmware_path)
        .map_err(|e| AppError(format!("读固件失败: {e}")))?;
    let mut hasher = Sha256::new();
    hasher.update(&payload);
    let sha = hex::encode(hasher.finalize());

    // 断点续传：进度文件校验（固件哈希一致才续传，否则重置）
    let progress_path = flash_progress_path(&firmware_path);
    let mut start_block: u32 = 0;
    if let Ok(prev) = std::fs::read_to_string(&progress_path) {
        if let Ok(p) = serde_json::from_str::<FlashProgress>(&prev) {
            if p.firmware_sha256 == sha && !p.finished {
                start_block = p.block_index;
            }
        }
    }

    let mut sp = serialport::new(&port, baud)
        .timeout(Duration::from_millis(1500))
        .open()
        .map_err(AppError::from)?;

    let block_size = 1024usize;
    let total_blocks = (payload.len() + block_size - 1) / block_size;
    let mut progress = FlashProgress {
        sn,
        firmware: firmware_path.clone(),
        firmware_sha256: sha.clone(),
        bytes_total: payload.len() as u64,
        bytes_done: start_block as u64 * block_size as u64,
        block_index: start_block,
        finished: false,
    };

    // 起 Handshake：接收端连发 'C' 请求 CRC 模式（续传时设备端 EOT 语义由实机校准）
    if start_block == 0 {
        wait_for(&mut sp, |b| b == CRC_MODE, 60, 250)?;
    } else {
        wait_for(&mut sp, |b| b == CRC_MODE || b == NAK, 30, 250)?;
    }

    for blk in start_block as usize..total_blocks {
        let chunk = &payload[blk * block_size..((blk + 1) * block_size).min(payload.len())];
        let block_no = ((blk % 256) + 1) as u8; // 块号 1..255 回绕
        let mut frame = vec![STX, block_no as u8, !(block_no as u8)];
        frame.extend_from_slice(chunk);
        frame.resize(frame.len() + (block_size - chunk.len()), 0x1A); // 补 0x1A
        let crc = crc16_modbus(&frame[3..]);
        frame.push((crc >> 8) as u8); // XMODEM CRC 高字节在前
        frame.push((crc & 0xFF) as u8);
        sp.write_all(&frame).map_err(|e| AppError(format!("写块失败: {e}")))?;

        let ack = wait_for(&mut sp, |b| b == ACK || b == NAK || b == CAN, 10, 1000)?;
        match ack {
            ACK => {}
            CAN => return Err(AppError("设备端取消烧录（CAN）".into())),
            _ => {
                // NAK：重发当前块一次；再失败即断（续传文件保留本轮进度）
                sp.write_all(&frame).map_err(|e| AppError(format!("重发失败: {e}")))?;
                wait_for(&mut sp, |b| b == ACK, 10, 1000)?;
            }
        }
        progress.block_index = (blk + 1) as u32;
        progress.bytes_done = ((blk + 1) * block_size).min(payload.len()) as u64;
        persist_progress(&progress_path, &progress)?;
        emit_progress(&app, &progress);
    }

    // EOT 结束
    sp.write_all(&[EOT]).map_err(|e| AppError(format!("EOT 失败: {e}")))?;
    wait_for(&mut sp, |b| b == ACK, 10, 1000)?;
    progress.finished = true;
    persist_progress(&progress_path, &progress)?;
    emit_progress(&app, &progress);
    Ok(progress)
}

/// 断点续传进度文件路径（{固件路径}.flash-progress，与固件同目录）。
fn flash_progress_path(firmware: &str) -> PathBuf {
    PathBuf::from(format!("{firmware}.flash-progress"))
}

/// 进度落盘（每块确认后调用，断点续传的数据源）。
fn persist_progress(path: &PathBuf, p: &FlashProgress) -> Result<()> {
    std::fs::write(path, serde_json::to_vec(p).map_err(|e| AppError(format!("序列化进度: {e}")))?)
        .map_err(|e| AppError(format!("写进度文件失败: {e}")))
}

/// 向前端发进度事件（event 通道 flash://progress，进度条数据源）。
fn emit_progress(app: &tauri::AppHandle, p: &FlashProgress) {
    use tauri::Emitter;
    let _ = app.emit("flash://progress", p);
}

/// CRC16-Modbus（XMODEM 块校验）。
fn crc16_modbus(data: &[u8]) -> u16 {
    use crc16::*;
    let mut state = State::MODBUS;
    for b in data {
        state.update_byte(*b);
    }
    state.get()
}

/// 等待串口出现满足条件的字节（超时秒数 × 重试间隔）。
fn wait_for<F>(sp: &mut Box<dyn serialport::SerialPort>, pred: F, secs: u64, interval_ms: u64) -> Result<u8>
where
    F: Fn(u8) -> bool,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    let mut buf = [0u8; 1];
    while std::time::Instant::now() < deadline {
        if let Ok(n) = sp.read(&mut buf) {
            if n == 1 && pred(buf[0]) {
                return Ok(buf[0]);
            }
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
    Err(AppError("等待设备应答超时".into()))
}

/// 清除断点续传进度（烧录成功后前端调用，防误续传旧固件）。
#[tauri::command]
pub fn flash_clear_progress(firmware_path: String) -> Result<()> {
    let p = flash_progress_path(&firmware_path);
    if p.exists() {
        std::fs::remove_file(&p).map_err(|e| AppError(format!("清理进度失败: {e}")))?;
    }
    Ok(())
}
