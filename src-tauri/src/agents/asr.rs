//! 语音转写（ASR）——纯 Rust，零 Tauri 类型（遵守 CLAUDE.md 解耦铁律 #1）。
//!
//! 统一走 **OpenAI 兼容 `/audio/transcriptions`（multipart）**：OpenAI / Groq /
//! 硅基流动等均兼容。配置从 `app_settings` 读取（[`AsrConfig::load`]）。转写结果是
//! **不可信外部输入**，由命令层统一过 `has_obvious_injection`，本文件不重复施加。

use anyhow::{anyhow, Result};
use std::time::Duration;

use crate::db::Db;

const SETTINGS_PROVIDER: &str = "asr.provider";
const SETTINGS_ENDPOINT: &str = "asr.endpoint";
const SETTINGS_MODEL: &str = "asr.model";
const SETTINGS_API_KEY: &str = "asr.api_key";
const SETTINGS_LANGUAGE: &str = "asr.language";

/// 已解析的 ASR 配置。`endpoint`+`model` 齐备才算启用。
#[derive(Debug, Clone)]
pub struct AsrConfig {
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    pub language: String,
}

impl AsrConfig {
    /// 从 app_settings 读取配置；缺失走默认值，api_key 解密。
    pub async fn load(db: &Db) -> Self {
        let provider = get_setting(db, SETTINGS_PROVIDER)
            .await
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let endpoint = get_setting(db, SETTINGS_ENDPOINT)
            .await
            .unwrap_or_default()
            .trim()
            .to_string();
        let model = get_setting(db, SETTINGS_MODEL)
            .await
            .unwrap_or_default()
            .trim()
            .to_string();
        let api_key = crate::core::secrets::decrypt(
            &get_setting(db, SETTINGS_API_KEY).await.unwrap_or_default(),
        )
        .unwrap_or_default();
        let language = get_setting(db, SETTINGS_LANGUAGE)
            .await
            .unwrap_or_default()
            .trim()
            .to_string();
        Self {
            provider,
            endpoint,
            model,
            api_key,
            language,
        }
    }

    /// 实际请求地址：endpoint 为空时按已知 provider 回退默认值。
    pub fn effective_endpoint(&self) -> String {
        if !self.endpoint.is_empty() {
            return self.endpoint.clone();
        }
        match self.provider.as_str() {
            "openai" => "https://api.openai.com/v1/audio/transcriptions".to_string(),
            "groq" => "https://api.groq.com/openai/v1/audio/transcriptions".to_string(),
            "siliconflow" => "https://api.siliconflow.cn/v1/audio/transcriptions".to_string(),
            _ => String::new(),
        }
    }

    /// 配置是否足以发起转写。
    pub fn is_enabled(&self) -> bool {
        !self.effective_endpoint().is_empty() && !self.model.is_empty()
    }
}

async fn get_setting(db: &Db, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key=?")
        .bind(key)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
}

/// 上传音频并返回转写文本。`filename` 需带正确扩展名（OpenAI 据此判定容器格式）。
pub async fn transcribe(cfg: &AsrConfig, audio: Vec<u8>, filename: &str, mime: &str) -> Result<String> {
    if !cfg.is_enabled() {
        return Err(anyhow!("语音转写未配置（缺 endpoint 或 model），请在设置 → 语音录入中配置"));
    }
    let url = cfg.effective_endpoint();

    let part = reqwest::multipart::Part::bytes(audio)
        .file_name(filename.to_string())
        .mime_str(mime)
        .map_err(|e| anyhow!("音频 MIME 非法：{}", e))?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", cfg.model.clone())
        .text("response_format", "json");
    if !cfg.language.is_empty() {
        form = form.text("language", cfg.language.clone());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| anyhow!(e))?;
    let mut req = client.post(&url).multipart(form);
    if !cfg.api_key.trim().is_empty() {
        req = req.bearer_auth(cfg.api_key.trim());
    }

    let resp = req.send().await.map_err(|e| anyhow!("ASR 请求失败：{}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let snippet: String = text.chars().take(300).collect();
        return Err(anyhow!("ASR 服务 HTTP {}: {}", status.as_u16(), snippet));
    }
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| anyhow!("ASR 响应非 JSON：{}", e))?;
    let out = value
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if out.is_empty() {
        return Err(anyhow!("ASR 未返回文本（音频可能为空或过短）"));
    }
    Ok(out)
}
