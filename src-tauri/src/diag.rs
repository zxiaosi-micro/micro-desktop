//! diag.rs · 本地诊断模块（S9-05，01 §8.6）：
//!   - 串口：JBD BMS 协议帧读基本信息寄存器 0x03（电压/电流/容量 SOC/温度）
//!   - CAN：SLCAN 串口协议适配 CAN 盒（帧收发骨架，实机校准）
//!   - BLE：btleplug 扫描 + JBD over BLE（Nordic UART 透传特征值，实机校准）
//!
//! 边界（02 §12.4）：**仅本地运行，数据不上云**——本模块只做协议帧解析与展示，
//! 不发起任何网络请求；采集记录导出走 transfer 模块本地落盘。

use crc16::*;
use serde::Serialize;

/// 统一错误（tauri command 序列化）。
#[derive(Debug, Serialize)]
pub struct AppError(pub String);

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AppError {}

impl From<serialport::Error> for AppError {
    fn from(e: serialport::Error) -> Self {
        AppError(format!("serialport: {e}"))
    }
}

type Result<T> = std::result::Result<T, AppError>;

// ---------------------------------------------------------------------------
// JBD 协议帧（常见 JBD/佳源 V3 口径；字段位宽以实机手册校准为准）
// ---------------------------------------------------------------------------

/// JBD 帧起始（SOH）
const JBD_START: u8 = 0x0D;
/// JBD 帧结束（EOH）
const JBD_END: u8 = 0x0A;
/// 基本信息寄存器
const REG_BASIC_INFO: u8 = 0x03;
/// 读命令 flag
const FLAG_READ: u8 = 0xA5;

/// 基本信息解析结果（寄存器 0x03）。
#[derive(Debug, Serialize)]
pub struct JbdBasicInfo {
    /// 总电压（V，0.01V 精度）
    pub voltage_v: f64,
    /// 电流（A，0.01A，负=放电）
    pub current_a: f64,
    /// 剩余容量（Ah，0.01Ah）
    pub remain_ah: f64,
    /// 额定容量（Ah，0.01Ah）
    pub rated_ah: f64,
    /// 循环次数
    pub cycles: u16,
    /// 生产日期（年/月/日）
    pub production: String,
    /// 均衡状态位图
    pub balance_bits: u16,
    /// 保护状态位图
    pub protect_bits: u16,
    /// SOC（%）
    pub soc: f64,
    /// FET 状态位图
    pub fet_bits: u16,
    /// 电芯电压串（mV）
    pub cell_mv: Vec<u16>,
    /// 温度串（0.1K 原始 → ℃）
    pub temps_c: Vec<f64>,
}

/// 构造 JBD 读帧：start flag cmd len data checksum-low checksum-high end。
pub fn jbd_read_frame(reg: u8, data_len: u8) -> Vec<u8> {
    let mut f = vec![JBD_START, FLAG_READ, reg, data_len];
    let crc = crc_modbus(&f);
    f.push((crc & 0xFF) as u8);
    f.push((crc >> 8) as u8);
    f.push(JBD_END);
    f
}

/// CRC16-Modbus（低字节在前）。
fn crc_modbus(data: &[u8]) -> u16 {
    let mut state = State::MODBUS;
    for b in data {
        state.update_byte(*b);
    }
    state.get()
}

/// 解析应答帧（start flag reg status len data... crc16 end）→ JbdBasicInfo。
pub fn parse_basic_info(frame: &[u8]) -> Result<JbdBasicInfo> {
    if frame.len() < 7 || frame[0] != JBD_START || *frame.last().unwrap() != JBD_END {
        return Err(AppError("JBD 帧边界不合法".into()));
    }
    if frame[3] != REG_BASIC_INFO {
        return Err(AppError(format!("非基本信息寄存器应答: 0x{:02X}", frame[3])));
    }
    let data = &frame[4..frame.len() - 3];
    if data.len() < 26 {
        return Err(AppError(format!("基本信息载荷过短: {} 字节", data.len())));
    }
    let u16_at = |i: usize| u16::from_be_bytes([data[i], data[i + 1]]);

    // 温度数量在固定 26 字节之后（协议可变长；逐帧解析前 N 个）
    let base = 26;
    // 剩余载荷按 2 字节切分为电芯电压（NTC 数量字段位置随固件差异，实机校准）
    let cell_count = (data.len() - base) / 2;
    let mut cell_mv = Vec::with_capacity(cell_count);
    for i in 0..cell_count {
        cell_mv.push(u16_at(base + i * 2));
    }
    // 温度紧跟电芯电压（0.1K 原始 → ℃）
    let mut temps_c = Vec::new();
    let mut i = base + cell_count * 2;
    while i + 2 <= data.len() {
        let raw = u16::from_be_bytes([data[i], data[i + 1]]);
        temps_c.push(raw as f64 / 10.0 - 273.15);
        i += 2;
    }

    Ok(JbdBasicInfo {
        voltage_v: u16_at(0) as f64 / 100.0,
        current_a: u16_at(2) as f64 / 100.0,
        remain_ah: u16_at(4) as f64 / 100.0,
        rated_ah: u16_at(6) as f64 / 100.0,
        cycles: u16_at(8),
        production: format!(
            "{:04}-{:02}-{:02}",
            2000 + (u16_at(10) >> 9),
            (u16_at(10) >> 5) & 0x0F,
            u16_at(10) & 0x1F
        ),
        balance_bits: u16_at(12),
        protect_bits: u16_at(14),
        soc: u16_at(16) as f64 / 100.0, // 部分 JBD 固件为 0.1% 精度——实机校准
        fet_bits: u16_at(20),
        cell_mv,
        temps_c,
    })
}

// ---------------------------------------------------------------------------
// tauri command：串口诊断
// ---------------------------------------------------------------------------

/// 扫描本机串口（CH340/CP210x 等 USB-串口桥）。
#[tauri::command]
pub fn list_serial_ports() -> Result<Vec<String>> {
    Ok(serialport::available_ports()
        .map_err(AppError::from)?
        .into_iter()
        .map(|p| match p {
            serialport::SerialPortType::UsbPort(info) => format!(
                "{} ({:04x}:{:04x})",
                p.port_name,
                info.vid,
                info.pid
            ),
            other => format!("{}", serial_name(&other)),
        })
        .collect())
}

fn serial_name(p: &serialport::SerialPortType) -> String {
    match p {
        serialport::SerialPortType::BluetoothPort => "Bluetooth".into(),
        serialport::SerialPortType::PciPort => "PCI".into(),
        serialport::SerialPortType::Unknown => "Unknown".into(),
        _ => "Other".into(),
    }
}

/// 读 JBD 基本信息寄存器（阻塞式：open → write 读帧 → read 应答 → 解析）。
#[tauri::command]
pub fn diag_serial_basic_info(port: String, baud: u32) -> Result<JbdBasicInfo> {
    let mut sp = serialport::new(&port, baud)
        .timeout(std::time::Duration::from_millis(800))
        .open()
        .map_err(AppError::from)?;
    use std::io::{Read, Write};
    let frame = jbd_read_frame(REG_BASIC_INFO, 0);
    sp.write_all(&frame).map_err(|e| AppError(format!("写帧失败: {e}")))?;
    let mut buf = [0u8; 128];
    let n = sp.read(&mut buf).map_err(|e| AppError(format!("读应答超时: {e}")))?;
    parse_basic_info(&buf[..n])
}

// ---------------------------------------------------------------------------
// CAN 盒（SLCAN 串口协议骨架；实机校准位时序与波特率）
// ---------------------------------------------------------------------------

/// SLCAN 读帧结果。
#[derive(Debug, Serialize)]
pub struct CanFrame {
    /// 11/29 位 ID（hex）
    pub id: String,
    /// 数据字节（hex 空格分隔）
    pub data: String,
}

/// 打开 SLCAN 通道并读取 N 帧（骨架：开放位速率命令留实机校准）。
#[tauri::command]
pub fn diag_slcan_read(port: String, baud: u32, count: u32) -> Result<Vec<CanFrame>> {
    let _ = (&port, &baud, &count);
    // 骨架：SLCAN 命令集（O=open, L=close, t/r=发标准帧）按盒型校准后启用；
    // 未校准前显式拒绝，避免误读帧被当作真实数据（ADR-19：协议标准+模拟器口径）。
    Err(AppError("SLCAN CAN 盒适配待实机校准（骨架占位，不返回伪数据）".into()))
}

// ---------------------------------------------------------------------------
// BLE（btleplug）：扫描与 JBD 透传骨架
// ---------------------------------------------------------------------------

/// BLE 外设扫描结果。
#[derive(Debug, Serialize)]
pub struct BleDevice {
    pub name: String,
    pub id: String,
    pub rssi: Option<i16>,
}

/// 扫描周边 BLE 外设 3 秒（JBD BMS 通常广播名含 "JBD"/"BT-"）。
#[tauri::command]
pub async fn diag_ble_scan() -> Result<Vec<BleDevice>> {
    use btleplug::api::{Central, Manager as _, ScanFilter};
    use btleplug::platform::Manager;

    let manager = Manager::new()
        .await
        .map_err(|e| AppError(format!("BLE manager: {e}")))?;
    let adapter = manager
        .adapters()
        .await
        .map_err(|e| AppError(format!("BLE adapters: {e}")))?
        .into_iter()
        .next()
        .ok_or_else(|| AppError("本机无可用 BLE 适配器".into()))?;
    adapter.start_scan(ScanFilter::default())
        .await
        .map_err(|e| AppError(format!("BLE scan: {e}")))?;
    tokio_time_sleep().await;
    let periph = adapter.peripherals()
        .await
        .map_err(|e| AppError(format!("BLE peripherals: {e}")))?;
    let mut out = Vec::new();
    for p in periph {
        if let Ok(Some(prop)) = p.properties().await {
            out.push(BleDevice {
                name: prop.local_name.unwrap_or_default(),
                id: p.id().to_string(),
                rssi: prop.rssi,
            });
        }
    }
    Ok(out)
}

// btleplug 是 async 的；tauri 2 runtime 即 tokio，直接用其 time 能力
async fn tokio_time_sleep() {
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
}

/// JBD over BLE 读基本信息（Nordic UART Service 6E400001-... 透传；实机校准 UUID）。
#[tauri::command]
pub async fn diag_ble_basic_info(_device_id: String) -> Result<JbdBasicInfo> {
    // 骨架：btleplug connect → discover_characteristics → write 读帧 / subscribe notify
    // 校准后启用；未校准前显式拒绝。
    Err(AppError("JBD over BLE 透传 UUID 待实机校准（骨架占位，不返回伪数据）".into()))
}
