//! M5 — preview data field-masking engine (design §5.3).
//!
//! Reads `preview.sensitive_fields` from a project's autoforge.yaml and applies
//! field-level masking (mask / hash / drop) to a seed preview SQLite database
//! before it is exposed. The statement builder is pure and unit-tested; the
//! apply step shells out to the `sqlite3` CLI (best-effort, degrades to no-op).

use serde::Deserialize;

pub const MASK_POLICY_VERSION: &str = "m5.1";

#[derive(Debug, Deserialize, Default)]
struct MaskYaml {
    preview: Option<PreviewSection>,
}
#[derive(Debug, Deserialize, Default)]
struct PreviewSection {
    #[serde(default)]
    sensitive_fields: Vec<SensitiveField>,
}
#[derive(Debug, Deserialize, Clone)]
struct SensitiveField {
    table: String,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default = "default_rule")]
    rule: String,
}
fn default_rule() -> String {
    "mask".to_string()
}

/// Parse `preview.sensitive_fields` from autoforge.yaml and build the masking SQL.
pub fn statements_from_yaml(config_yaml: &str) -> Vec<String> {
    let cfg: MaskYaml = match serde_yaml::from_str(config_yaml) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let fields = cfg
        .preview
        .map(|p| p.sensitive_fields)
        .unwrap_or_default();
    build_mask_statements(&fields)
}

fn build_mask_statements(specs: &[SensitiveField]) -> Vec<String> {
    let mut out = Vec::new();
    for spec in specs {
        let table = sanitize_ident(&spec.table);
        if table.is_empty() {
            continue;
        }
        for field in &spec.fields {
            let f = sanitize_ident(field);
            if f.is_empty() {
                continue;
            }
            let stmt = match spec.rule.as_str() {
                // mask: keep first char, replace the rest — format-preserving-ish placeholder
                "mask" => format!(
                    "UPDATE \"{table}\" SET \"{f}\" = substr(CAST(\"{f}\" AS TEXT),1,1) || '****' WHERE \"{f}\" IS NOT NULL;"
                ),
                // hash: irreversible random token (uniqueness not guaranteed, but unlinkable)
                "hash" => format!(
                    "UPDATE \"{table}\" SET \"{f}\" = 'h:' || substr(lower(hex(randomblob(16))),1,16) WHERE \"{f}\" IS NOT NULL;"
                ),
                // drop: clear entirely
                "drop" => format!("UPDATE \"{table}\" SET \"{f}\" = NULL;"),
                // unknown rule → conservative mask
                _ => format!(
                    "UPDATE \"{table}\" SET \"{f}\" = '***' WHERE \"{f}\" IS NOT NULL;"
                ),
            };
            out.push(stmt);
        }
    }
    out
}

/// Only allow identifier-safe characters; reject anything that could break out of
/// the quoted identifier (defence-in-depth even though these come from local yaml).
fn sanitize_ident(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// Apply masking statements to a SQLite db file via the `sqlite3` CLI.
/// Returns the number of statements applied; 0 if sqlite3 is unavailable.
pub async fn apply_to_sqlite(db_path: &str, statements: &[String]) -> usize {
    if statements.is_empty() {
        return 0;
    }
    let script = statements.join("\n");
    let result = tokio::process::Command::new("sqlite3")
        .arg(db_path)
        .arg(&script)
        .output()
        .await;
    match result {
        Ok(out) if out.status.success() => statements.len(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(table: &str, fields: &[&str], rule: &str) -> SensitiveField {
        SensitiveField {
            table: table.to_string(),
            fields: fields.iter().map(|s| s.to_string()).collect(),
            rule: rule.to_string(),
        }
    }

    #[test]
    fn mask_keeps_first_char() {
        let s = build_mask_statements(&[fields("users", &["phone"], "mask")]);
        assert_eq!(s.len(), 1);
        assert!(s[0].contains("UPDATE \"users\""));
        assert!(s[0].contains("substr(CAST(\"phone\" AS TEXT),1,1)"));
    }

    #[test]
    fn drop_sets_null() {
        let s = build_mask_statements(&[fields("customers", &["id_card"], "drop")]);
        assert_eq!(s, vec!["UPDATE \"customers\" SET \"id_card\" = NULL;".to_string()]);
    }

    #[test]
    fn hash_is_irreversible_token() {
        let s = build_mask_statements(&[fields("users", &["email"], "hash")]);
        assert!(s[0].contains("randomblob"));
        assert!(s[0].contains("'h:'"));
    }

    #[test]
    fn multiple_fields_expand() {
        let s = build_mask_statements(&[fields("users", &["phone", "email"], "mask")]);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn sql_injection_in_ident_is_stripped() {
        let s = build_mask_statements(&[fields("users\"; DROP TABLE users;--", &["x"], "drop")]);
        // identifier sanitized — no stray quote/semicolon/dash survives in the table name
        assert!(s[0].starts_with("UPDATE \"usersDROPTABLEusers\""));
    }

    #[test]
    fn parses_from_yaml() {
        let yaml = r#"
preview:
  sensitive_fields:
    - table: users
      fields: [phone, email]
      rule: mask
    - table: customers
      fields: [id_card]
      rule: drop
"#;
        let stmts = statements_from_yaml(yaml);
        assert_eq!(stmts.len(), 3);
    }

    #[test]
    fn empty_yaml_yields_no_statements() {
        assert!(statements_from_yaml("project:\n  name: x\n").is_empty());
    }

    /// 运行配置真源是 `.autoforge/run-config.json`（JSON）。JSON 是合法 YAML，
    /// 所以 serde_yaml 消费者必须能直接解析 JSON 形态的配置。
    #[test]
    fn parses_json_run_config() {
        let json = r#"{"preview":{"sensitive_fields":[{"table":"users","fields":["phone","email"],"rule":"mask"}]}}"#;
        let stmts = statements_from_yaml(json);
        assert_eq!(stmts.len(), 2);
    }

    /// Real runtime test: build a SQLite db, apply masking via the sqlite3 CLI,
    /// and verify the stored value was actually masked. Skips if sqlite3 absent.
    #[tokio::test]
    async fn apply_masks_a_real_sqlite_db() {
        let has_sqlite3 = std::process::Command::new("sqlite3")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !has_sqlite3 {
            return; // environment has no sqlite3 CLI — skip
        }
        let path = std::env::temp_dir().join(format!("af_mask_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let url = format!("sqlite://{}?mode=rwc", path.display());

        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect(&url)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE users (phone TEXT)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO users (phone) VALUES ('13800138000')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await; // release the file for the sqlite3 CLI

        let stmts = statements_from_yaml(
            "preview:\n  sensitive_fields:\n    - table: users\n      fields: [phone]\n      rule: mask\n",
        );
        let applied = apply_to_sqlite(&path.to_string_lossy(), &stmts).await;
        assert_eq!(applied, 1);

        let pool2 = sqlx::sqlite::SqlitePoolOptions::new()
            .connect(&url)
            .await
            .unwrap();
        let (val,): (String,) = sqlx::query_as("SELECT phone FROM users")
            .fetch_one(&pool2)
            .await
            .unwrap();
        pool2.close().await;
        assert_eq!(val, "1****", "phone should be masked to first char + ****");
        let _ = std::fs::remove_file(&path);
    }
}
