//! transfer.rs · 文件传输模块（S9-05）：巡检/诊断记录导出 CSV/XLSX + 日志 zip 打包。
//!
//! 边界：纯本地文件加工（01 §8.6"CSV/XLSX 导出与日志打包"）；落盘目录走
//! dialog 选点 + capabilities fs scoped（$DOWNLOAD/$TEMP），不触网。

use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;

use crate::diag::AppError;

type Result<T> = std::result::Result<T, AppError>;

/// 导出行（前端把表格数据按行传入；列头与值均为字符串——展示层口径）。
#[derive(Debug, Serialize)]
pub struct ExportRequest {
    /// 文件名建议（不含扩展名）
    pub filename: String,
    /// 列头
    pub headers: Vec<String>,
    /// 行数据（与 headers 等宽）
    pub rows: Vec<Vec<String>>,
}

/// 导出结果。
#[derive(Debug, Serialize)]
pub struct ExportResult {
    pub path: String,
    pub rows: usize,
}

/// 导出 CSV（UTF-8 BOM——Excel 中文兼容）。
#[tauri::command]
pub fn export_csv(request: ExportRequest, target_dir: String) -> Result<ExportResult> {
    let path: PathBuf = PathBuf::from(target_dir)
        .join(format!("{}.csv", request.filename));
    let mut w = csv::Writer::from_path(&path)
        .map_err(|e| AppError(format!("建 CSV 失败: {e}")))?;
    // BOM
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(&path)
        .map_err(|e| AppError(format!("写 CSV 失败: {e}")))?;
    file.write_all(&[0xEF, 0xBB, 0xBF]).ok();
    drop(file);
    w.write_record(&request.headers)
        .map_err(|e| AppError(format!("写表头失败: {e}")))?;
    for row in &request.rows {
        w.write_record(row)
            .map_err(|e| AppError(format!("写行失败: {e}")))?;
    }
    w.flush().map_err(|e| AppError(format!("刷盘失败: {e}")))?;
    Ok(ExportResult {
        path: path.to_string_lossy().into_owned(),
        rows: request.rows.len(),
    })
}

/// 导出 XLSX（rust_xlsxwriter；首行表头加粗）。
#[tauri::command]
pub fn export_xlsx(request: ExportRequest, target_dir: String) -> Result<ExportResult> {
    let path = PathBuf::from(target_dir).join(format!("{}.xlsx", request.filename));
    let mut wb = rust_xlsxwriter::Workbook::new();
    let sheet = wb.add_worksheet();
    let bold = rust_xlsxwriter::Format::new().set_bold();
    for (c, h) in request.headers.iter().enumerate() {
        sheet
            .write_with_format(0, c as u16, h, &bold)
            .map_err(|e| AppError(format!("写表头失败: {e}")))?;
    }
    for (r, row) in request.rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            sheet
                .write(r as u32 + 1, c as u16, v)
                .map_err(|e| AppError(format!("写行失败: {e}")))?;
        }
    }
    wb.save(&path).map_err(|e| AppError(format!("保存 XLSX 失败: {e}")))?;
    Ok(ExportResult {
        path: path.to_string_lossy().into_owned(),
        rows: request.rows.len(),
    })
}

/// 日志打包 zip（输入文件路径清单 → 单 zip；deflate）。
#[tauri::command]
pub fn pack_logs(file_paths: Vec<String>, zip_path: String) -> Result<ExportResult> {
    let file = std::fs::File::create(&zip_path)
        .map_err(|e| AppError(format!("建 zip 失败: {e}")))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut packed = 0usize;
    for p in &file_paths {
        let meta = std::path::Path::new(p);
        let name = meta
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("entry-{packed}"));
        let body = std::fs::read(p).map_err(|e| AppError(format!("读 {p} 失败: {e}")))?;
        zip.start_file(name, opts.clone())
            .map_err(|e| AppError(format!("打包 {p} 失败: {e}")))?;
        std::io::copy(&mut body.as_slice(), &mut zip)
            .map_err(|e| AppError(format!("写入 {p} 失败: {e}")))?;
        packed += 1;
    }
    zip.finish().map_err(|e| AppError(format!("收尾 zip 失败: {e}")))?;
    Ok(ExportResult { path: zip_path, rows: packed })
}
