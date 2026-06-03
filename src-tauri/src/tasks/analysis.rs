use crate::core::{event, security};
use crate::db::Db;
use anyhow::{anyhow, Result};
use tracing::{error, info};
use uuid::Uuid;

pub async fn run(db: &Db, app: &tauri::AppHandle, issue_id: &str) -> Result<()> {
    // Load issue
    let issue = sqlx::query_as::<_, crate::models::issue::Issue>("SELECT * FROM issues WHERE id=?")
        .bind(issue_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| anyhow!("issue {} not found", issue_id))?;

    // Security check
    if security::has_obvious_injection(&issue.title)
        || security::has_obvious_injection(&issue.description)
    {
        error!(
            "issue {} failed security check — possible injection",
            issue_id
        );
        sqlx::query("UPDATE issues SET status='rejected', updated_at=datetime('now') WHERE id=?")
            .bind(issue_id)
            .execute(db)
            .await?;
        return Err(anyhow!("security check failed for issue {}", issue_id));
    }

    // Layer 1 (design §4.3): deeper LLM sanitization beyond the regex fast-reject.
    // Internal sources (scan/monitor) are trusted and skip this; gracefully
    // degrades to "allow" when the claude CLI is unavailable.
    if issue.source_type != "scan" && issue.source_type != "monitor" {
        let combined = format!("{}\n{}", issue.title, issue.description);
        if !crate::agents::local_claude::safety_check(&combined).await {
            error!("issue {} rejected by Layer 1 LLM sanitizer", issue_id);
            sqlx::query(
                "UPDATE issues SET status='rejected', updated_at=datetime('now') WHERE id=?",
            )
            .bind(issue_id)
            .execute(db)
            .await?;
            return Err(anyhow!("layer 1 sanitizer rejected issue {}", issue_id));
        }
    }

    info!("analyzing issue: {} — {}", issue_id, issue.title);

    // Run analysis
    let result = crate::agents::analysis::analyze(&issue.title, &issue.description)
        .await
        .unwrap_or_default();

    // Persist analysis
    let analysis_id = Uuid::new_v4().to_string();
    let affected_modules_json = serde_json::to_string(&result.affected_modules)?;
    sqlx::query(
        "INSERT OR REPLACE INTO issue_analyses
         (id, issue_id, authenticity_score, feasibility_score, priority_suggestion,
          category_suggestion, severity_suggestion, affected_modules, analysis_summary, raw_llm_output)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&analysis_id)
    .bind(&issue.id)
    .bind(result.authenticity_score)
    .bind(result.feasibility_score)
    .bind(result.priority_suggestion)
    .bind(&result.category_suggestion)
    .bind(&result.severity_suggestion)
    .bind(&affected_modules_json)
    .bind(&result.analysis_summary)
    .bind(&result.raw_output)
    .execute(db)
    .await?;

    // Update issue status and promote analysis suggestions into queue fields.
    sqlx::query(
        "UPDATE issues
         SET status='pending_review_1',
             priority=?,
             category=?,
             severity=?,
             updated_at=datetime('now')
         WHERE id=?",
    )
    .bind(result.priority_suggestion)
    .bind(&result.category_suggestion)
    .bind(&result.severity_suggestion)
    .bind(&issue.id)
    .execute(db)
    .await?;

    // Emit events
    event::emit(
        app,
        event::AppEvent::AnalysisCompleted {
            issue_id: issue.id.clone(),
        },
    );

    event::emit(
        app,
        event::AppEvent::ReviewNeeded {
            cr_id: String::new(),
            issue_title: issue.title.clone(),
            stage: 1,
        },
    );

    info!("analysis completed for issue {}", issue_id);
    Ok(())
}
