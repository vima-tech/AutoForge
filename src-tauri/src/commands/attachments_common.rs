//! 附件通用助手：MIME/大小/类型白名单校验、文件名消毒、存储路径解析。
//!
//! 纯函数，零 Tauri 类型——会议室附件（`conversation_attachments`）与需求附件
//! （`issue_attachments`）共用同一套安全策略，避免两份白名单漂移。
//! 白名单只放只读/可内联的安全类型，显式拒绝脚本、HTML、SVG、压缩包、可执行文件。

use std::path::{Component, Path, PathBuf};

/// 附件大小硬上限（10 MB）。base64 载荷与解码后字节都按此校验。
pub const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;

/// 通过白名单校验后得出的规范化策略：落盘扩展名、规范 MIME、kind（image/file）。
#[derive(Debug)]
pub struct AttachmentPolicy {
    pub ext: &'static str,
    pub mime: &'static str,
    pub kind: &'static str,
}

/// 把 `rel_path`（形如 `<owner_id>/<uuid>.<ext>`）解析为 `attachments_base()` 下的
/// 绝对路径，拒绝任何非 Normal 组件（`..`、绝对前缀等），防路径越界。
pub fn attachment_path_from_rel(rel_path: &str) -> Result<PathBuf, String> {
    let rel = Path::new(rel_path);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("附件路径无效".to_string());
    }
    Ok(PathBuf::from(crate::state::attachments_base()).join(rel))
}

/// 消毒原始文件名：剥目录、替非法字符、去首尾空格/点、截断 120 字符。空则回落 "attachment"。
pub fn sanitize_file_name(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("attachment");
    let mut cleaned = String::with_capacity(base.len());
    for ch in base.chars() {
        if ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            cleaned.push('_');
        } else {
            cleaned.push(ch);
        }
    }
    let cleaned = cleaned
        .trim_matches([' ', '.'])
        .chars()
        .take(120)
        .collect::<String>();
    if cleaned.is_empty() {
        "attachment".to_string()
    } else {
        cleaned
    }
}

/// 按扩展名 + 内容魔数 + 浏览器 MIME 提示三方一致校验附件，返回规范化策略。
/// 三者不一致或类型不在白名单一律拒绝。
pub fn validate_attachment(
    name: &str,
    mime_hint: &str,
    bytes: &[u8],
) -> Result<AttachmentPolicy, String> {
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let policy = match ext.as_str() {
        "png" if is_png(bytes) => AttachmentPolicy {
            ext: "png",
            mime: "image/png",
            kind: "image",
        },
        "jpg" | "jpeg" if is_jpeg(bytes) => AttachmentPolicy {
            ext: "jpg",
            mime: "image/jpeg",
            kind: "image",
        },
        "webp" if is_webp(bytes) => AttachmentPolicy {
            ext: "webp",
            mime: "image/webp",
            kind: "image",
        },
        "gif" if is_gif(bytes) => AttachmentPolicy {
            ext: "gif",
            mime: "image/gif",
            kind: "image",
        },
        "pdf" if bytes.starts_with(b"%PDF-") => AttachmentPolicy {
            ext: "pdf",
            mime: "application/pdf",
            kind: "file",
        },
        "txt" | "log" if is_safe_text(bytes) => AttachmentPolicy {
            ext: "txt",
            mime: "text/plain",
            kind: "file",
        },
        "md" if is_safe_text(bytes) => AttachmentPolicy {
            ext: "md",
            mime: "text/markdown",
            kind: "file",
        },
        "csv" if is_safe_text(bytes) => AttachmentPolicy {
            ext: "csv",
            mime: "text/csv",
            kind: "file",
        },
        "json"
            if is_safe_text(bytes)
                && serde_json::from_slice::<serde_json::Value>(bytes).is_ok() =>
        {
            AttachmentPolicy {
                ext: "json",
                mime: "application/json",
                kind: "file",
            }
        }
        "yaml" | "yml" if is_safe_text(bytes) => AttachmentPolicy {
            ext: "yaml",
            mime: "application/x-yaml",
            kind: "file",
        },
        "toml" if is_safe_text(bytes) => AttachmentPolicy {
            ext: "toml",
            mime: "application/toml",
            kind: "file",
        },
        _ => {
            return Err(
                "不支持的附件类型。允许：PNG/JPG/WebP/GIF/PDF/TXT/MD/JSON/CSV/YAML/TOML，禁止脚本、HTML、SVG、压缩包和可执行文件。"
                    .to_string(),
            );
        }
    };

    if !mime_hint.is_empty() && !mime_hint_matches(mime_hint, policy.mime) {
        return Err("文件扩展名、浏览器 MIME 和内容特征不一致".to_string());
    }

    Ok(policy)
}

fn mime_hint_matches(hint: &str, canonical: &str) -> bool {
    let hint = hint
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if hint.is_empty() || hint == "application/octet-stream" {
        return true;
    }
    matches!(
        (hint.as_str(), canonical),
        ("image/jpeg", "image/jpeg")
            | ("image/png", "image/png")
            | ("image/webp", "image/webp")
            | ("image/gif", "image/gif")
            | ("application/pdf", "application/pdf")
            | ("text/plain", "text/plain")
            | ("text/plain", "text/markdown")
            | ("text/markdown", "text/markdown")
            | ("text/csv", "text/csv")
            | ("application/json", "application/json")
            | ("text/yaml", "application/x-yaml")
            | ("application/x-yaml", "application/x-yaml")
            | ("application/toml", "application/toml")
    )
}

fn is_safe_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 3 && bytes[0..3] == [0xff, 0xd8, 0xff]
}

fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
}
