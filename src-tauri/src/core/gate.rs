//! §7 gate downgrade — let low-risk changes skip human review 2.
//!
//! A change "class" (derived by the grader from the diff) earns trust through a
//! clean streak of human approvals at review 2. Once the streak crosses the
//! threshold, T0/T1 changes of that class auto-merge without human review; any
//! reject/revision resets the streak. Hard-floor (T3) classes never auto-pass,
//! and the whole feature is globally OFF by default — trust is earned, not given.

use crate::db::Db;

/// Clean-approval streak required to move a class to `eligible`.
const ELIGIBLE_THRESHOLD: i64 = 10;
/// Clean-approval streak required to move a class to `auto`.
const AUTO_THRESHOLD: i64 = 20;

pub async fn auto_pass_enabled(db: &Db) -> bool {
    sqlx::query_as::<_, (String,)>("SELECT value FROM app_settings WHERE key='auto_pass_enabled'")
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .map(|(v,)| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub async fn set_auto_pass_enabled(db: &Db, on: bool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES ('auto_pass_enabled', ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now')",
    )
    .bind(if on { "1" } else { "0" })
    .execute(db)
    .await?;
    Ok(())
}

/// Global switch: on a pre-merge conflict, automatically hand the conflict to the
/// code agent to resolve (then re-test and route back to review 2). OFF by default.
/// Co-located with `auto_pass_enabled` in the Settings 门控降级 panel.
pub async fn auto_conflict_resolve_enabled(db: &Db) -> bool {
    sqlx::query_as::<_, (String,)>(
        "SELECT value FROM app_settings WHERE key='auto_conflict_resolve_enabled'",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .map(|(v,)| v == "1" || v.eq_ignore_ascii_case("true"))
    .unwrap_or(false)
}

pub async fn set_auto_conflict_resolve_enabled(db: &Db, on: bool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES ('auto_conflict_resolve_enabled', ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now')",
    )
    .bind(if on { "1" } else { "0" })
    .execute(db)
    .await?;
    Ok(())
}

/// Global switch: let the human edit the merge commit message at code review.
/// OFF by default — when off, merges always use the `AutoForge merge: <id>` template
/// and the Audit page hides the input. Co-located with the other merge-pipeline flags.
pub async fn custom_merge_message_enabled(db: &Db) -> bool {
    sqlx::query_as::<_, (String,)>(
        "SELECT value FROM app_settings WHERE key='custom_merge_message_enabled'",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .map(|(v,)| v == "1" || v.eq_ignore_ascii_case("true"))
    .unwrap_or(false)
}

pub async fn set_custom_merge_message_enabled(db: &Db, on: bool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES ('custom_merge_message_enabled', ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now')",
    )
    .bind(if on { "1" } else { "0" })
    .execute(db)
    .await?;
    Ok(())
}

/// Whether a graded change may skip human review 2.
/// Requires: globally enabled, tier T0/T1 (never T2/T3), and the class is `auto`.
pub async fn should_auto_pass(db: &Db, tier: &str, change_class: &str) -> bool {
    if !matches!(tier, "T0" | "T1") {
        return false;
    }
    if !auto_pass_enabled(db).await {
        return false;
    }
    let state: Option<(String,)> =
        sqlx::query_as("SELECT trust_state FROM auto_pass_policy WHERE change_class=?")
            .bind(change_class)
            .fetch_optional(db)
            .await
            .unwrap_or(None);
    state.map(|(s,)| s).as_deref() == Some("auto")
}

/// Record a human review-2 outcome for a change class and recompute its trust.
/// A clean approval extends the streak; any reject/revision resets it to cold.
pub async fn record_review_outcome(db: &Db, change_class: &str, approved: bool) {
    let _ = sqlx::query("INSERT OR IGNORE INTO auto_pass_policy (change_class) VALUES (?)")
        .bind(change_class)
        .execute(db)
        .await;

    if !approved {
        let _ = sqlx::query(
            "UPDATE auto_pass_policy
             SET reject_count=reject_count+1, approve_count=0, trust_state='cold', updated_at=datetime('now')
             WHERE change_class=?",
        )
        .bind(change_class)
        .execute(db)
        .await;
        return;
    }

    let _ = sqlx::query(
        "UPDATE auto_pass_policy
         SET approve_count=approve_count+1, updated_at=datetime('now')
         WHERE change_class=?",
    )
    .bind(change_class)
    .execute(db)
    .await;

    let streak = sqlx::query_as::<_, (i64,)>(
        "SELECT approve_count FROM auto_pass_policy WHERE change_class=?",
    )
    .bind(change_class)
    .fetch_optional(db)
    .await
    .unwrap_or(None)
    .map(|(c,)| c)
    .unwrap_or(0);

    let new_state = if streak >= AUTO_THRESHOLD {
        "auto"
    } else if streak >= ELIGIBLE_THRESHOLD {
        "eligible"
    } else {
        "cold"
    };
    let _ = sqlx::query("UPDATE auto_pass_policy SET trust_state=? WHERE change_class=?")
        .bind(new_state)
        .bind(change_class)
        .execute(db)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> Db {
        let p = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE auto_pass_policy (change_class TEXT PRIMARY KEY, trust_state TEXT NOT NULL DEFAULT 'cold', approve_count INTEGER NOT NULL DEFAULT 0, reject_count INTEGER NOT NULL DEFAULT 0, updated_at TEXT)",
        )
        .execute(&p)
        .await
        .unwrap();
        sqlx::query("CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT)")
            .execute(&p)
            .await
            .unwrap();
        p
    }

    #[tokio::test]
    async fn trust_earned_then_reset_on_reject() {
        let db = pool().await;
        set_auto_pass_enabled(&db, true).await.unwrap();
        assert!(!should_auto_pass(&db, "T1", "src").await, "未达阈值不应自动过");
        for _ in 0..AUTO_THRESHOLD {
            record_review_outcome(&db, "src", true).await;
        }
        assert!(should_auto_pass(&db, "T1", "src").await, "达阈值后应自动过");
        assert!(!should_auto_pass(&db, "T3", "src").await, "T3 硬地板永不自动过");
        record_review_outcome(&db, "src", false).await; // a reject resets the streak
        assert!(!should_auto_pass(&db, "T1", "src").await, "退改后清零,不再自动过");
    }

    #[tokio::test]
    async fn globally_disabled_blocks_auto() {
        let db = pool().await;
        for _ in 0..AUTO_THRESHOLD {
            record_review_outcome(&db, "src", true).await;
        }
        // global flag is OFF (never enabled)
        assert!(!should_auto_pass(&db, "T0", "src").await, "全局关闭时不自动过");
    }
}
