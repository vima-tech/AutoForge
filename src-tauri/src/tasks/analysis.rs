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
    // Trusted internal sources skip the LLM check; external sources always run it.
    // Gracefully degrades to "allow" when the claude CLI is unavailable.
    // "github" is intentionally NOT trusted: GitHub issue text is externally
    // authored (anyone can open an issue) and must run the LLM sanitizer.
    const TRUSTED_SOURCES: &[&str] = &[
        "scan", "monitor", "todo_scan", "security_audit", "ci_monitor",
    ];
    if !TRUSTED_SOURCES.contains(&issue.source_type.as_str()) {
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

    // Load project context from local repo path
    let repo_path: Option<String> = sqlx::query_as::<_, crate::models::project::Project>(
        "SELECT * FROM projects WHERE id=?",
    )
    .bind(&issue.project_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .filter(|p| !p.repo_path.is_empty())
    .map(|p| p.repo_path);

    let project_context: Option<String> = if let Some(path) = repo_path {
        let ctx = crate::agents::analysis::build_project_context(&path).await;
        if ctx.trim().is_empty() { None } else { Some(ctx) }
    } else {
        None
    };

    // Run analysis (uses the analysis Agent's bound custom LLM; CLI only as fallback)
    let result = crate::agents::analysis::analyze(
        db,
        &issue.title,
        &issue.description,
        project_context.as_deref(),
    )
    .await
    .unwrap_or_default();

    // Persist analysis
    let analysis_id = Uuid::new_v4().to_string();
    let affected_modules_json = serde_json::to_string(&result.affected_modules)?;
    sqlx::query(
        "INSERT OR REPLACE INTO issue_analyses
         (id, issue_id, authenticity_score, feasibility_score, priority_suggestion,
          category_suggestion, severity_suggestion, affected_modules, analysis_summary,
          raw_llm_output, analysis_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
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
    .bind(&result.analysis_json)
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
