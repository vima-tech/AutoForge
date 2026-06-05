use super::IntakePayload;

/// 解析批量导入内容，返回 IntakePayload 列表
/// format: "text" | "csv" | "json"
pub fn parse(project_id: &str, format: &str, content: &str) -> Result<Vec<IntakePayload>, String> {
    match format {
        "text" => parse_text(project_id, content),
        "csv" => parse_csv(project_id, content),
        "json" => parse_json(project_id, content),
        _ => Err(format!("不支持的格式: {}，请使用 text / csv / json", format)),
    }
}

fn parse_text(project_id: &str, content: &str) -> Result<Vec<IntakePayload>, String> {
    let payloads = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .take(200)
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
    let mut lines = content.lines().peekable();

    // 检测并解析表头
    let mut col_title: usize = 0;
    let mut col_desc: Option<usize> = None;
    let mut col_cat: Option<usize> = None;
    let mut col_sev: Option<usize> = None;
    let mut has_header = false;

    if let Some(first) = lines.peek() {
        let cells: Vec<String> = first.split(',').map(|c| c.trim().to_lowercase()).collect();
        if cells.iter().any(|c| c == "title" || c == "标题") {
            has_header = true;
            for (i, c) in cells.iter().enumerate() {
                match c.as_str() {
                    "title" | "标题" => col_title = i,
                    "description" | "描述" => col_desc = Some(i),
                    "category" | "类型" => col_cat = Some(i),
                    "severity" | "严重级" => col_sev = Some(i),
                    _ => {}
                }
            }
        }
    }
    if has_header {
        lines.next();
    }

    let payloads = lines
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .take(200)
        .map(|line| {
            let cells: Vec<String> = line.split(',').map(|c| c.trim().to_string()).collect();
            let cell = |idx: usize| -> Option<String> {
                cells.get(idx).map(|s| s.to_string()).filter(|s| !s.is_empty())
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
        .collect();
    Ok(payloads)
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
        .take(200)
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
