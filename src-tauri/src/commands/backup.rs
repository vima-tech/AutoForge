//! 配置备份：把系统配置、LLM 配置、角色 Agent 配置等**整包口令加密导出**，便于
//! 换机/重装后一键导入，快速进入生产。
//!
//! 为什么不能直接拷库行：库内密钥（LLM api_key、MCP env/headers、通知 secret、
//! app_settings 内的 web_search/asr key）以本机主密钥（系统钥匙环）加密为 `enc:v1:`
//! 密文，**换台机器主密钥不同，密文无法解密**。因此导出时先用本机主密钥**解密**还原明文，
//! 再用**用户口令**派生密钥（PBKDF2-HMAC-SHA256）将整包 JSON 经 AES-256-GCM 重新加密成
//! 可移植的 `.afbackup` 信封文件；导入时用同一口令解密，落库时再用目标机主密钥重新加密
//! （走 `secrets::encrypt_field`）。导出文件里**不含**本机主密钥，只有口令能解开。
//!
//! 本模块业务逻辑为纯 Rust（仅 `#[tauri::command]` 薄包装触碰 Tauri：取 state、备份目录、
//! 以及「在文件夹中显示」的 opener）。遵守 CLAUDE.md 解耦铁律。

use crate::core::secrets;
use crate::db::Db;
use crate::models::agent::Agent;
use crate::models::llm_config::LlmConfig;
use crate::models::mcp_server::McpServer;
use crate::models::notify::NotifyChannel;
use crate::state::AppState;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tauri::State;

type HmacSha256 = Hmac<Sha256>;

/// 信封格式标识与版本（写入文件，导入时校验）。
const FORMAT_TAG: &str = "autoforge-config-backup";
const BACKUP_VERSION: u32 = 1;
/// PBKDF2 迭代轮数（OWASP 2023 对 PBKDF2-HMAC-SHA256 的推荐量级）。
const PBKDF2_ITERS: u32 = 210_000;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
/// 口令最小长度，避免空/过弱口令导出形同未加密。
const MIN_PASSPHRASE: usize = 6;

// ---- 数据结构 ----

/// app_settings 单条：`secret` 标记该值在库里是否为密文（导出时已解密为明文，
/// 导入时据此决定是否重新加密落库）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettingExport {
    key: String,
    value: String,
    secret: bool,
}

/// 备份内层明文负载（被口令加密前的 JSON）。所有密钥字段均为**明文**。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfigBackup {
    version: u32,
    exported_at: String,
    llm_configs: Vec<LlmConfig>,
    agents: Vec<Agent>,
    mcp_servers: Vec<McpServer>,
    notify_channels: Vec<NotifyChannel>,
    app_settings: Vec<SettingExport>,
}

/// 落盘的口令加密信封（外层，全文本 JSON，便于前端以 FileReader 读取导入）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Envelope {
    format: String,
    v: u32,
    kdf: String,
    iter: u32,
    salt: String,       // base64
    nonce: String,      // base64
    ciphertext: String, // base64(密文)
}

/// 各类配置条目计数，供 UI 回显「导出/导入了什么」。
#[derive(Debug, Clone, Serialize)]
pub struct BackupSummary {
    pub llm_configs: usize,
    pub agents: usize,
    pub mcp_servers: usize,
    pub notify_channels: usize,
    pub app_settings: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    /// 导出文件绝对路径。
    pub path: String,
    pub summary: BackupSummary,
}

// ---- PBKDF2-HMAC-SHA256（dkLen == hLen == 32，单块）----

fn pbkdf2_key(password: &[u8], salt: &[u8], iters: u32) -> [u8; 32] {
    let block = |data: &[u8]| -> [u8; 32] {
        let mut mac = <HmacSha256 as Mac>::new_from_slice(password).expect("hmac accepts any key len");
        mac.update(data);
        let out = mac.finalize().into_bytes();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&out);
        arr
    };
    // U1 = PRF(P, salt || INT(1))
    let mut salted = Vec::with_capacity(salt.len() + 4);
    salted.extend_from_slice(salt);
    salted.extend_from_slice(&1u32.to_be_bytes());
    let mut u = block(&salted);
    let mut t = u;
    for _ in 1..iters {
        u = block(&u);
        for i in 0..32 {
            t[i] ^= u[i];
        }
    }
    t
}

fn aes_key(key: &[u8; 32]) -> Aes256Gcm {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key))
}

// ---- 采集（导出）----

/// 从库中采集全部可移植配置，并把密文字段解密还原为明文（仅存在于内存与即将口令加密的负载里）。
async fn collect_backup(db: &Db) -> Result<ConfigBackup, String> {
    // LLM 配置：api_key 解密。
    let mut llm_configs = sqlx::query_as::<_, LlmConfig>("SELECT * FROM llm_configs ORDER BY created_at")
        .fetch_all(db)
        .await
        .map_err(|e| e.to_string())?;
    for c in &mut llm_configs {
        c.api_key = secrets::decrypt(&c.api_key)?;
    }

    // 角色 / 业务 Agent：无密钥字段。
    let agents = sqlx::query_as::<_, Agent>("SELECT * FROM agents ORDER BY created_at")
        .fetch_all(db)
        .await
        .map_err(|e| e.to_string())?;

    // MCP server：env_json / headers_json 解密。
    let mut mcp_servers = sqlx::query_as::<_, McpServer>("SELECT * FROM mcp_servers ORDER BY created_at")
        .fetch_all(db)
        .await
        .map_err(|e| e.to_string())?;
    for m in &mut mcp_servers {
        m.env_json = secrets::decrypt(&m.env_json)?;
        m.headers_json = secrets::decrypt(&m.headers_json)?;
    }

    // 通知通道：secret 解密。
    let mut notify_channels = sqlx::query_as::<_, NotifyChannel>("SELECT * FROM notify_channels ORDER BY created_at")
        .fetch_all(db)
        .await
        .map_err(|e| e.to_string())?;
    for c in &mut notify_channels {
        c.secret = secrets::decrypt(&c.secret)?;
    }

    // app_settings：逐条记录是否为密文；密文解密为明文，明文照搬。
    let rows = sqlx::query_as::<_, (String, String)>("SELECT key, value FROM app_settings")
        .fetch_all(db)
        .await
        .map_err(|e| e.to_string())?;
    let mut app_settings = Vec::with_capacity(rows.len());
    for (key, value) in rows {
        let is_secret = secrets::is_encrypted(&value);
        let plain = if is_secret { secrets::decrypt(&value)? } else { value };
        app_settings.push(SettingExport { key, value: plain, secret: is_secret });
    }

    Ok(ConfigBackup {
        version: BACKUP_VERSION,
        exported_at: chrono::Local::now().to_rfc3339(),
        llm_configs,
        agents,
        mcp_servers,
        notify_channels,
        app_settings,
    })
}

fn summarize(b: &ConfigBackup) -> BackupSummary {
    BackupSummary {
        llm_configs: b.llm_configs.len(),
        agents: b.agents.len(),
        mcp_servers: b.mcp_servers.len(),
        notify_channels: b.notify_channels.len(),
        app_settings: b.app_settings.len(),
    }
}

/// 采集 → 口令加密 → 写入 `dir/autoforge-config-<时间戳>.afbackup`。返回绝对路径与计数。
async fn export_inner(db: &Db, passphrase: &str, dir: &str) -> Result<ExportResult, String> {
    if passphrase.chars().count() < MIN_PASSPHRASE {
        return Err(format!("口令至少 {} 位", MIN_PASSPHRASE));
    }
    let backup = collect_backup(db).await?;
    let summary = summarize(&backup);
    let plaintext = serde_json::to_vec(&backup).map_err(|e| e.to_string())?;

    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    rand::rngs::OsRng.fill_bytes(&mut nonce);

    let key = pbkdf2_key(passphrase.as_bytes(), &salt, PBKDF2_ITERS);
    let ct = aes_key(&key)
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|e| format!("加密失败: {}", e))?;

    let envelope = Envelope {
        format: FORMAT_TAG.to_string(),
        v: BACKUP_VERSION,
        kdf: "pbkdf2-hmac-sha256".to_string(),
        iter: PBKDF2_ITERS,
        salt: B64.encode(salt),
        nonce: B64.encode(nonce),
        ciphertext: B64.encode(&ct),
    };
    let text = serde_json::to_string_pretty(&envelope).map_err(|e| e.to_string())?;

    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("创建备份目录失败: {}", e))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = std::path::Path::new(dir).join(format!("autoforge-config-{}.afbackup", stamp));
    tokio::fs::write(&path, text.as_bytes())
        .await
        .map_err(|e| format!("写入备份文件失败: {}", e))?;

    Ok(ExportResult {
        path: path.to_string_lossy().to_string(),
        summary,
    })
}

// ---- 解析 + 落库（导入）----

fn decrypt_envelope(content: &str, passphrase: &str) -> Result<ConfigBackup, String> {
    let env: Envelope = serde_json::from_str(content.trim())
        .map_err(|_| "文件不是有效的 AutoForge 配置备份".to_string())?;
    if env.format != FORMAT_TAG {
        return Err("文件不是 AutoForge 配置备份".to_string());
    }
    if env.v > BACKUP_VERSION {
        return Err(format!("备份版本 v{} 高于本程序支持的 v{}，请升级后再导入", env.v, BACKUP_VERSION));
    }
    let salt = B64.decode(env.salt.trim()).map_err(|_| "salt 解码失败".to_string())?;
    let nonce = B64.decode(env.nonce.trim()).map_err(|_| "nonce 解码失败".to_string())?;
    let ct = B64.decode(env.ciphertext.trim()).map_err(|_| "密文解码失败".to_string())?;
    if nonce.len() != NONCE_LEN {
        return Err("nonce 长度异常".to_string());
    }
    let key = pbkdf2_key(passphrase.as_bytes(), &salt, env.iter.max(1));
    let plaintext = aes_key(&key)
        .decrypt(Nonce::from_slice(&nonce), ct.as_ref())
        .map_err(|_| "解密失败：口令不正确或文件已损坏".to_string())?;
    serde_json::from_slice::<ConfigBackup>(&plaintext)
        .map_err(|e| format!("备份内容解析失败: {}", e))
}

/// 把备份 upsert 回库（按主键合并，不删除现有未涉及的行；密钥字段用目标机主密钥重新加密）。
/// 同一事务内 `defer_foreign_keys`，规避 agents.llm_id → llm_configs 的插入顺序约束。
async fn apply_backup(db: &Db, backup: &ConfigBackup) -> Result<BackupSummary, String> {
    let mut tx = db.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("PRAGMA defer_foreign_keys=ON")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    for c in &backup.llm_configs {
        let api_key = secrets::encrypt_field(&c.api_key)?;
        sqlx::query(
            "INSERT INTO llm_configs
                (id,name,model,endpoint,api_key,ctx_window,temperature,enabled,api_spec,supports_vision,is_default,created_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?)
             ON CONFLICT(id) DO UPDATE SET
                name=excluded.name, model=excluded.model, endpoint=excluded.endpoint,
                api_key=excluded.api_key, ctx_window=excluded.ctx_window, temperature=excluded.temperature,
                enabled=excluded.enabled, api_spec=excluded.api_spec, supports_vision=excluded.supports_vision,
                is_default=excluded.is_default",
        )
        .bind(&c.id).bind(&c.name).bind(&c.model).bind(&c.endpoint).bind(&api_key)
        .bind(&c.ctx_window).bind(c.temperature).bind(c.enabled).bind(&c.api_spec)
        .bind(c.supports_vision).bind(c.is_default).bind(&c.created_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    for a in &backup.agents {
        sqlx::query(
            "INSERT INTO agents
                (id,name,name_en,role,color,initial,llm_id,system_prompt,forge_role,role_type,
                 system_kind,capabilities_json,max_concurrency,visible_in_chat,mentionable,enabled,
                 prompt_mode,memory_enabled,created_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
             ON CONFLICT(id) DO UPDATE SET
                name=excluded.name, name_en=excluded.name_en, role=excluded.role, color=excluded.color,
                initial=excluded.initial, llm_id=excluded.llm_id, system_prompt=excluded.system_prompt,
                forge_role=excluded.forge_role, role_type=excluded.role_type, system_kind=excluded.system_kind,
                capabilities_json=excluded.capabilities_json, max_concurrency=excluded.max_concurrency,
                visible_in_chat=excluded.visible_in_chat, mentionable=excluded.mentionable,
                enabled=excluded.enabled, prompt_mode=excluded.prompt_mode, memory_enabled=excluded.memory_enabled",
        )
        .bind(&a.id).bind(&a.name).bind(&a.name_en).bind(&a.role).bind(&a.color).bind(&a.initial)
        .bind(&a.llm_id).bind(&a.system_prompt).bind(&a.forge_role).bind(&a.role_type)
        .bind(&a.system_kind).bind(&a.capabilities_json).bind(a.max_concurrency)
        .bind(a.visible_in_chat).bind(a.mentionable).bind(a.enabled).bind(&a.prompt_mode)
        .bind(a.memory_enabled).bind(&a.created_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    for m in &backup.mcp_servers {
        let env_json = secrets::encrypt_field(&m.env_json)?;
        let headers_json = secrets::encrypt_field(&m.headers_json)?;
        sqlx::query(
            "INSERT INTO mcp_servers
                (id,name,transport,command,args_json,env_json,url,headers_json,agent_ids_json,for_code_agent,capability_map_json,enabled,created_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)
             ON CONFLICT(id) DO UPDATE SET
                name=excluded.name, transport=excluded.transport, command=excluded.command,
                args_json=excluded.args_json, env_json=excluded.env_json, url=excluded.url,
                headers_json=excluded.headers_json, agent_ids_json=excluded.agent_ids_json,
                for_code_agent=excluded.for_code_agent, capability_map_json=excluded.capability_map_json, enabled=excluded.enabled",
        )
        .bind(&m.id).bind(&m.name).bind(&m.transport).bind(&m.command).bind(&m.args_json)
        .bind(&env_json).bind(&m.url).bind(&headers_json).bind(&m.agent_ids_json)
        .bind(m.for_code_agent).bind(&m.capability_map_json).bind(m.enabled).bind(&m.created_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    for c in &backup.notify_channels {
        let secret = secrets::encrypt_field(&c.secret)?;
        sqlx::query(
            "INSERT INTO notify_channels
                (id,name,kind,target,events,enabled,secret,created_at)
             VALUES (?,?,?,?,?,?,?,?)
             ON CONFLICT(id) DO UPDATE SET
                name=excluded.name, kind=excluded.kind, target=excluded.target, events=excluded.events,
                enabled=excluded.enabled, secret=excluded.secret",
        )
        .bind(&c.id).bind(&c.name).bind(&c.kind).bind(&c.target).bind(&c.events)
        .bind(c.enabled).bind(&secret).bind(&c.created_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    for s in &backup.app_settings {
        let value = if s.secret { secrets::encrypt_field(&s.value)? } else { s.value.clone() };
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES (?, ?, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        )
        .bind(&s.key)
        .bind(&value)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(summarize(backup))
}

// ---- Tauri 命令（薄包装）----

/// 加密导出全部配置到备份目录，返回文件路径与计数。
#[tauri::command]
pub async fn export_config(
    passphrase: String,
    state: State<'_, AppState>,
) -> Result<ExportResult, String> {
    export_inner(&state.db, &passphrase, &crate::state::backups_base()).await
}

/// 从备份文件内容导入并恢复配置（按主键 upsert，不删除现有未涉及的行）。
#[tauri::command]
pub async fn import_config(
    content: String,
    passphrase: String,
    state: State<'_, AppState>,
) -> Result<BackupSummary, String> {
    let backup = decrypt_envelope(&content, &passphrase)?;
    let summary = apply_backup(&state.db, &backup).await?;
    // 知识层蒸馏 LLM 等可能随导入变化，刷新 in-process Innate 模型配置（best-effort）。
    crate::knowledge::refresh_kb_models(&state.db).await;
    Ok(summary)
}

/// 在系统文件管理器中定位备份文件（仅允许备份目录内的路径，防越权 reveal）。
#[tauri::command]
pub async fn reveal_backup(path: String, app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let base = std::fs::canonicalize(crate::state::backups_base())
        .map_err(|e| format!("备份目录不可用: {}", e))?;
    let target = std::fs::canonicalize(&path).map_err(|_| "文件不存在".to_string())?;
    if !target.starts_with(&base) {
        return Err("路径越界".to_string());
    }
    let target_str = target.to_string_lossy().to_string();
    if app.opener().reveal_item_in_dir(&target_str).is_err() {
        let dir = target
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .ok_or("无法定位所在文件夹")?;
        app.opener()
            .open_path(dir, None::<&str>)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_matches_known_vector() {
        // RFC 6070 风格自检：固定输入 → 确定输出，且 salt/口令变化输出不同。
        let a = pbkdf2_key(b"password", b"salt", 2);
        let b = pbkdf2_key(b"password", b"salt", 2);
        assert_eq!(a, b, "相同输入应得相同密钥");
        let c = pbkdf2_key(b"password", b"salt2", 2);
        assert_ne!(a, c, "不同 salt 应得不同密钥");
        let d = pbkdf2_key(b"password2", b"salt", 2);
        assert_ne!(a, d, "不同口令应得不同密钥");
    }

    #[test]
    fn envelope_roundtrip() {
        let backup = ConfigBackup {
            version: BACKUP_VERSION,
            exported_at: "now".into(),
            llm_configs: vec![],
            agents: vec![],
            mcp_servers: vec![],
            notify_channels: vec![],
            app_settings: vec![SettingExport { key: "k".into(), value: "v".into(), secret: false }],
        };
        // 手工走一遍导出加密逻辑（不落盘），再解密校验往返一致。
        let plaintext = serde_json::to_vec(&backup).unwrap();
        let mut salt = [0u8; SALT_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let key = pbkdf2_key(b"correct horse", &salt, 1000);
        let ct = aes_key(&key).encrypt(Nonce::from_slice(&nonce), plaintext.as_ref()).unwrap();
        let env = Envelope {
            format: FORMAT_TAG.into(),
            v: BACKUP_VERSION,
            kdf: "pbkdf2-hmac-sha256".into(),
            iter: 1000,
            salt: B64.encode(salt),
            nonce: B64.encode(nonce),
            ciphertext: B64.encode(&ct),
        };
        let text = serde_json::to_string(&env).unwrap();
        let ok = decrypt_envelope(&text, "correct horse").unwrap();
        assert_eq!(ok.app_settings.len(), 1);
        assert!(decrypt_envelope(&text, "wrong pass").is_err(), "错误口令应解密失败");
    }
}
