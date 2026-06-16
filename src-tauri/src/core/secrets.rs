//! 凭据静态加密（信封加密 / envelope encryption）。
//!
//! 设计：操作系统密钥链（macOS Keychain / Windows Credential Manager /
//! Linux Secret Service）只保管**一把 32 字节主密钥**；各类密钥（LLM api_key、
//! MCP env/headers、web_search api_key）用该主密钥经 **AES-256-GCM** 加密后，
//! 以 `enc:v1:<base64(nonce||ciphertext)>` 密文形式落 SQLite。直接打开 .db 文件
//! 看不到任何明文密钥。
//!
//! 兜底：无桌面钥匙环的 Linux（headless/server）取不到 Secret Service 时，主密钥
//! 退化保存到 app 数据目录下的 `master.key`（0600 权限）。安全性弱于系统钥匙环，
//! 通过 [`backend`] 暴露给设置页提示用户。
//!
//! 本模块是**纯 Rust，零 Tauri 类型**（遵守 CLAUDE.md 解耦铁律 #1）：主密钥文件
//! 路径经 [`init_secrets`] 的 `OnceLock` 注入，迁移函数只依赖 `db::Db`。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use std::sync::OnceLock;

/// 密文前缀（含版本号，便于未来轮换算法）。
const PREFIX: &str = "enc:v1:";
const KEYRING_SERVICE: &str = "autoforge";
const KEYRING_USER: &str = "master-key";
const NONCE_LEN: usize = 12;

/// 当前主密钥所在后端，供 UI 提示。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretBackend {
    /// 操作系统密钥链。
    Keychain,
    /// 0600 本地文件兜底（无系统钥匙环时）。
    File,
}

static MASTER_KEY: OnceLock<[u8; 32]> = OnceLock::new();
static FALLBACK_PATH: OnceLock<String> = OnceLock::new();
static BACKEND: OnceLock<SecretBackend> = OnceLock::new();

/// 注入主密钥兜底文件的绝对路径（在 Tauri setup 中以 app 数据目录拼出）。
/// 必须在首次加解密前调用；非 Tauri 入口同样可调用。
pub fn init_secrets(master_key_file: String) {
    let _ = FALLBACK_PATH.set(master_key_file);
}

/// 预热：立即加载/创建主密钥并确定后端，避免首个加解密调用阻塞在钥匙环 IPC 上，
/// 同时让 [`backend`] 在 UI 查询前就绪。
pub fn warm_up() {
    let _ = master_key();
}

/// 当前主密钥后端（未初始化时按 keychain 上报，仅用于显示）。
pub fn backend() -> SecretBackend {
    *BACKEND.get().unwrap_or(&SecretBackend::Keychain)
}

/// 判断字符串是否为本模块产出的密文。
pub fn is_encrypted(s: &str) -> bool {
    s.starts_with(PREFIX)
}

/// 加密任意明文，产出 `enc:v1:` 前缀的密文。空串原样返回（保持「未设置」语义）。
pub fn encrypt(plaintext: &str) -> Result<String, String> {
    if plaintext.is_empty() {
        return Ok(String::new());
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(master_key()));
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
        .map_err(|e| format!("加密失败: {}", e))?;
    let mut blob = Vec::with_capacity(NONCE_LEN + ct.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ct);
    Ok(format!("{}{}", PREFIX, B64.encode(blob)))
}

/// 解密 `enc:v1:` 密文；非密文（旧明文 / 空串）原样透传，保证迁移期读路径稳健。
pub fn decrypt(stored: &str) -> Result<String, String> {
    let Some(rest) = stored.strip_prefix(PREFIX) else {
        return Ok(stored.to_string());
    };
    let blob = B64.decode(rest).map_err(|e| format!("密文 base64 解码失败: {}", e))?;
    if blob.len() <= NONCE_LEN {
        return Err("密文长度异常".into());
    }
    let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(master_key()));
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|e| format!("解密失败（主密钥不匹配或数据损坏）: {}", e))?;
    String::from_utf8(pt).map_err(|e| format!("明文非 UTF-8: {}", e))
}

/// 存储侧统一入口：空串与平凡 JSON 空对象 `{}`（无密钥可言）保持可读，其余加密。
/// 已是密文则幂等返回。
pub fn encrypt_field(s: &str) -> Result<String, String> {
    if s.is_empty() || s == "{}" || is_encrypted(s) {
        return Ok(s.to_string());
    }
    encrypt(s)
}

// ---- 主密钥加载 ----

fn master_key() -> &'static [u8; 32] {
    MASTER_KEY.get_or_init(load_or_create_master_key)
}

fn load_or_create_master_key() -> [u8; 32] {
    if let Some(key) = keychain_get_or_create() {
        let _ = BACKEND.set(SecretBackend::Keychain);
        return key;
    }
    let key = file_get_or_create();
    let _ = BACKEND.set(SecretBackend::File);
    key
}

/// 尝试从系统钥匙环取主密钥；不存在则生成并写回。钥匙环不可用（无 Secret
/// Service / 被锁 / 平台错误）时返回 `None`，由调用方走文件兜底。
fn keychain_get_or_create() -> Option<[u8; 32]> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()?;
    match entry.get_password() {
        Ok(b64) => decode_key(b64.trim()),
        Err(keyring::Error::NoEntry) => {
            let key = random_key();
            entry.set_password(&B64.encode(key)).ok()?;
            Some(key)
        }
        Err(_) => None,
    }
}

fn file_get_or_create() -> [u8; 32] {
    let path = FALLBACK_PATH
        .get()
        .cloned()
        .unwrap_or_else(|| "/tmp/autoforge-master.key".to_string());
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Some(k) = decode_key(data.trim()) {
            return k;
        }
    }
    let key = random_key();
    if let Err(e) = write_key_file(&path, &key) {
        eprintln!("[secrets] 主密钥文件写入失败（密钥仅存于内存，重启后失效）: {}", e);
    }
    key
}

fn write_key_file(path: &str, key: &[u8; 32]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(B64.encode(key).as_bytes())
}

fn decode_key(b64: &str) -> Option<[u8; 32]> {
    let bytes = B64.decode(b64).ok()?;
    <[u8; 32]>::try_from(bytes.as_slice()).ok()
}

fn random_key() -> [u8; 32] {
    let mut k = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut k);
    k
}

// ---- 一次性迁移：把库内残留明文密钥就地加密 ----

/// 启动时调用：扫描 `llm_configs.api_key`、`mcp_servers.env_json/headers_json`、
/// `app_settings` 内 `web_search.api_key`，把仍为明文的值就地加密。幂等（已是密文
/// 或平凡值则跳过）。返回搬迁的字段数。
pub async fn migrate_plaintext_secrets(db: &crate::db::Db) -> anyhow::Result<usize> {
    let mut migrated = 0usize;

    // LLM api_key
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, api_key FROM llm_configs")
        .fetch_all(db)
        .await?;
    for (id, key) in rows {
        if key.is_empty() || is_encrypted(&key) {
            continue;
        }
        let enc = encrypt(&key).map_err(|e| anyhow::anyhow!(e))?;
        sqlx::query("UPDATE llm_configs SET api_key=? WHERE id=?")
            .bind(enc)
            .bind(id)
            .execute(db)
            .await?;
        migrated += 1;
    }

    // MCP env_json / headers_json
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, env_json, headers_json FROM mcp_servers")
            .fetch_all(db)
            .await?;
    for (id, env, headers) in rows {
        let new_env = encrypt_field(&env).map_err(|e| anyhow::anyhow!(e))?;
        let new_headers = encrypt_field(&headers).map_err(|e| anyhow::anyhow!(e))?;
        if new_env != env || new_headers != headers {
            sqlx::query("UPDATE mcp_servers SET env_json=?, headers_json=? WHERE id=?")
                .bind(new_env)
                .bind(new_headers)
                .bind(id)
                .execute(db)
                .await?;
            migrated += 1;
        }
    }

    // web_search.api_key（app_settings KV）
    let ws: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key='web_search.api_key'")
            .fetch_optional(db)
            .await?;
    if let Some(key) = ws {
        if !key.is_empty() && !is_encrypted(&key) {
            let enc = encrypt(&key).map_err(|e| anyhow::anyhow!(e))?;
            sqlx::query("UPDATE app_settings SET value=? WHERE key='web_search.api_key'")
                .bind(enc)
                .execute(db)
                .await?;
            migrated += 1;
        }
    }

    Ok(migrated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encrypt_decrypt() {
        let secret = "sk-ant-very-secret-key-1234567890";
        let ct = encrypt(secret).unwrap();
        assert!(is_encrypted(&ct), "应带 enc:v1: 前缀");
        assert!(!ct.contains(secret), "密文不得含明文");
        assert_eq!(decrypt(&ct).unwrap(), secret);
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(encrypt("").unwrap(), "");
        assert_eq!(encrypt_field("").unwrap(), "");
        assert_eq!(encrypt_field("{}").unwrap(), "{}");
    }

    #[test]
    fn legacy_plaintext_passes_through() {
        // 非 enc:v1: 前缀的旧明文，decrypt 原样返回，保证迁移期读路径不炸。
        assert_eq!(decrypt("plain-legacy-key").unwrap(), "plain-legacy-key");
        assert_eq!(decrypt("{}").unwrap(), "{}");
    }

    #[test]
    fn encrypt_field_is_idempotent() {
        let once = encrypt_field("secret-value").unwrap();
        let twice = encrypt_field(&once).unwrap();
        assert_eq!(once, twice, "已是密文应幂等返回");
    }

    #[test]
    fn distinct_nonces_produce_distinct_ciphertexts() {
        let a = encrypt("same").unwrap();
        let b = encrypt("same").unwrap();
        assert_ne!(a, b, "随机 nonce 应使同一明文每次密文不同");
        assert_eq!(decrypt(&a).unwrap(), decrypt(&b).unwrap());
    }
}
