use super::IntakePayload;

/// 单次批量导入的条数上限（防止误导入超大表）。
const MAX_ROWS: usize = 200;

/// 解析批量导入内容（文本/粘贴渠道），返回 IntakePayload 列表。
/// format: "text" | "csv" | "json"
pub fn parse(project_id: &str, format: &str, content: &str) -> Result<Vec<IntakePayload>, String> {
    match format {
        "text" => parse_text(project_id, content),
        "csv" => parse_csv(project_id, content),
        "json" => parse_json(project_id, content),
        _ => Err(format!("不支持的格式: {}，请使用 text / csv / json", format)),
    }
}

/// 解析批量导入文件（上传渠道），按文件名后缀分流。
/// 支持 .csv / .xlsx / .xls / .ods；csv 按 UTF-8 文本处理，其余走电子表格解析。
pub fn parse_file(
    project_id: &str,
    file_name: &str,
    bytes: &[u8],
) -> Result<Vec<IntakePayload>, String> {
    let ext = file_name
        .rsplit('.')
        .next()
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "csv" | "txt" => {
            let text = String::from_utf8(bytes.to_vec())
                .map_err(|_| "CSV 文件不是有效的 UTF-8 文本".to_string())?;
            parse_csv(project_id, &text)
        }
        "xlsx" | "xls" | "xlsm" | "ods" => parse_spreadsheet(project_id, bytes, &ext),
        other => Err(format!(
            "不支持的文件类型: .{}，请使用 .csv / .xlsx / .xls",
            other
        )),
    }
}

fn parse_text(project_id: &str, content: &str) -> Result<Vec<IntakePayload>, String> {
    let payloads = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .take(MAX_ROWS)
        .map(|line| IntakePayload {
            project_id: project_id.to_string(),
            title: line.to_string(),
            description: None,
            category: None,
            severity: None,
            source_type: "bulk".to_string(),
            source_ref: None,
        })
        .collect();
    Ok(payloads)
}

fn parse_csv(project_id: &str, content: &str) -> Result<Vec<IntakePayload>, String> {
    let rows: Vec<Vec<String>> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(split_csv_line)
        .collect();
    Ok(rows_to_payloads(project_id, rows))
}

/// 极简 CSV 行切分：支持双引号包裹字段（含逗号、转义双引号 ""）。
fn split_csv_line(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                cells.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn parse_spreadsheet(
    project_id: &str,
    bytes: &[u8],
    ext: &str,
) -> Result<Vec<IntakePayload>, String> {
    use calamine::{Data, Reader};
    use std::io::Cursor;

    let cursor = Cursor::new(bytes.to_vec());
    // 取第一个工作表的全部单元格。
    let range = match ext {
        "xls" => {
            let mut wb = calamine::Xls::new(cursor)
                .map_err(|e| format!("解析 .xls 失败: {}", e))?;
            first_sheet_range(&mut wb)?
        }
        "ods" => {
            let mut wb = calamine::Ods::new(cursor)
                .map_err(|e| format!("解析 .ods 失败: {}", e))?;
            first_sheet_range(&mut wb)?
        }
        _ => {
            let mut wb = calamine::Xlsx::new(cursor)
                .map_err(|e| format!("解析 .xlsx 失败: {}", e))?;
            first_sheet_range(&mut wb)?
        }
    };

    let rows: Vec<Vec<String>> = range
        .rows()
        .map(|row| {
            row.iter()
                .map(|cell| match cell {
                    Data::Empty => String::new(),
                    Data::String(s) => s.trim().to_string(),
                    Data::Float(f) => {
                        // 整数显示为整数，避免 "1" 变 "1.0"
                        if f.fract() == 0.0 {
                            format!("{}", *f as i64)
                        } else {
                            f.to_string()
                        }
                    }
                    Data::Int(i) => i.to_string(),
                    Data::Bool(b) => b.to_string(),
                    Data::DateTime(d) => d.to_string(),
                    other => other.to_string(),
                })
                .collect()
        })
        .collect();
    Ok(rows_to_payloads(project_id, rows))
}

fn first_sheet_range<R: calamine::Reader<std::io::Cursor<Vec<u8>>>>(
    wb: &mut R,
) -> Result<calamine::Range<calamine::Data>, String>
where
    <R as calamine::Reader<std::io::Cursor<Vec<u8>>>>::Error: std::fmt::Display,
{
    let name = wb
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| "电子表格不含任何工作表".to_string())?;
    wb.worksheet_range(&name)
        .map_err(|e| format!("读取工作表失败: {}", e))
}

/// 表格行 → 载荷：检测表头（title/标题…）做列映射；无表头则首列即标题。
fn rows_to_payloads(project_id: &str, rows: Vec<Vec<String>>) -> Vec<IntakePayload> {
    let mut rows = rows.into_iter().filter(|r| r.iter().any(|c| !c.is_empty()));

    let mut col_title: usize = 0;
    let mut col_desc: Option<usize> = None;
    let mut col_cat: Option<usize> = None;
    let mut col_sev: Option<usize> = None;

    let first = match rows.next() {
        Some(r) => r,
        None => return vec![],
    };

    let header_cells: Vec<String> = first.iter().map(|c| c.trim().to_lowercase()).collect();
    let has_header = header_cells.iter().any(|c| c == "title" || c == "标题");

    let mut data_rows: Vec<Vec<String>> = Vec::new();
    if has_header {
        for (i, c) in header_cells.iter().enumerate() {
            match c.as_str() {
                "title" | "标题" => col_title = i,
                "description" | "描述" => col_desc = Some(i),
                "category" | "类型" | "分类" => col_cat = Some(i),
                "severity" | "严重级" | "严重程度" | "优先级" => col_sev = Some(i),
                _ => {}
            }
        }
    } else {
        // 首行也是数据
        data_rows.push(first);
    }
    data_rows.extend(rows);

    data_rows
        .into_iter()
        .take(MAX_ROWS)
        .map(|cells| {
            let cell = |idx: usize| -> Option<String> {
                cells
                    .get(idx)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            };
            IntakePayload {
                project_id: project_id.to_string(),
                title: cell(col_title).unwrap_or_default(),
                description: col_desc.and_then(cell),
                category: col_cat.and_then(cell),
                severity: col_sev.and_then(cell),
                source_type: "bulk".to_string(),
                source_ref: None,
            }
        })
        .filter(|p| !p.title.is_empty())
        .collect()
}

fn parse_json(project_id: &str, content: &str) -> Result<Vec<IntakePayload>, String> {
    let val: serde_json::Value =
        serde_json::from_str(content).map_err(|e| format!("JSON 解析失败: {}", e))?;

    let arr = if let serde_json::Value::Array(a) = val {
        a
    } else if val.is_object() {
        vec![val]
    } else {
        return Err("JSON 格式错误：需要对象数组或单个对象".to_string());
    };

    let payloads = arr
        .into_iter()
        .take(MAX_ROWS)
        .filter_map(|obj| {
            let title = obj.get("title")?.as_str()?.trim().to_string();
            if title.is_empty() {
                return None;
            }
            Some(IntakePayload {
                project_id: project_id.to_string(),
                title,
                description: obj
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                category: obj
                    .get("category")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                severity: obj
                    .get("severity")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                source_type: "bulk".to_string(),
                source_ref: None,
            })
        })
        .collect();
    Ok(payloads)
}

// ── 模板导出 ──────────────────────────────────────────────────────────────────

/// 模板列头（与解析端列名保持一致）与一行示例。
const TEMPLATE_HEADERS: [&str; 4] = ["title", "description", "category", "severity"];
const TEMPLATE_SAMPLE: [&str; 4] = ["添加用户头像上传功能", "用户可在个人资料页上传并裁剪头像", "Feature", "medium"];
const TEMPLATE_SAMPLE2: [&str; 4] = ["修复登录超时 Bug", "连续操作 30 分钟后会话异常断开", "Bug", "high"];

/// 生成批量导入模板文件字节。
/// format: "csv" | "xlsx" → 返回 (建议文件名, MIME, 字节)。
pub fn template_bytes(format: &str) -> Result<(String, String, Vec<u8>), String> {
    match format {
        "csv" => {
            let mut s = String::new();
            s.push_str(&TEMPLATE_HEADERS.join(","));
            s.push('\n');
            s.push_str(&TEMPLATE_SAMPLE.join(","));
            s.push('\n');
            s.push_str(&TEMPLATE_SAMPLE2.join(","));
            s.push('\n');
            // 加 UTF-8 BOM，Excel 打开 CSV 不乱码。
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(s.as_bytes());
            Ok((
                "autoforge-需求批量导入模板.csv".to_string(),
                "text/csv".to_string(),
                bytes,
            ))
        }
        "xlsx" => {
            let bytes = build_xlsx_template()?;
            Ok((
                "autoforge-需求批量导入模板.xlsx".to_string(),
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(),
                bytes,
            ))
        }
        other => Err(format!("不支持的模板格式: {}，请使用 csv / xlsx", other)),
    }
}

fn build_xlsx_template() -> Result<Vec<u8>, String> {
    use rust_xlsxwriter::{Format, Workbook};

    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet().set_name("需求").map_err(|e| e.to_string())?;

    let header_fmt = Format::new().set_bold().set_background_color(0xE8772E);

    for (col, h) in TEMPLATE_HEADERS.iter().enumerate() {
        sheet
            .write_string_with_format(0, col as u16, *h, &header_fmt)
            .map_err(|e| e.to_string())?;
    }
    for (col, v) in TEMPLATE_SAMPLE.iter().enumerate() {
        sheet
            .write_string(1, col as u16, *v)
            .map_err(|e| e.to_string())?;
    }
    for (col, v) in TEMPLATE_SAMPLE2.iter().enumerate() {
        sheet
            .write_string(2, col as u16, *v)
            .map_err(|e| e.to_string())?;
    }
    // 合理列宽
    sheet.set_column_width(0, 28).ok();
    sheet.set_column_width(1, 40).ok();
    sheet.set_column_width(2, 14).ok();
    sheet.set_column_width(3, 12).ok();

    workbook.save_to_buffer().map_err(|e| e.to_string())
}
