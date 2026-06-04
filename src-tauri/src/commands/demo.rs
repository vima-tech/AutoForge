use crate::state::AppState;
use tauri::State;
use tokio::process::Command;

#[tauri::command]
pub async fn open_url(url: String, app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_shell::ShellExt;
    app.shell().open(&url, None).map_err(|e| e.to_string())
}

async fn git(repo: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "AutoForge Demo")
        .env("GIT_AUTHOR_EMAIL", "demo@autoforge.ai")
        .env("GIT_COMMITTER_NAME", "AutoForge Demo")
        .env("GIT_COMMITTER_EMAIL", "demo@autoforge.ai")
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("git {:?} failed: {}", args, err));
    }
    Ok(())
}

fn write(path: &str, content: &str) -> Result<(), String> {
    std::fs::write(path, content).map_err(|e| e.to_string())
}

fn mkdir(path: &str) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| e.to_string())
}

/// 填充审计页演示数据：
/// 1. 创建 /tmp/autoforge-demo 真实 git 仓库，各 CR 分支有真实 diff
/// 2. 创建 worktree 目录（使路径存在检查通过）
/// 3. 更新测试项目 repo_path
/// 4. 插入 preview_environments
#[tauri::command]
pub async fn seed_demo_data(state: State<'_, AppState>) -> Result<String, String> {
    let repo = "/tmp/autoforge-demo";

    // --- 1. 初始化 git 仓库 ---
    let _ = std::fs::remove_dir_all(repo);
    mkdir(repo)?;
    git(repo, &["init", "-b", "main"]).await?;
    git(repo, &["config", "user.email", "demo@autoforge.ai"]).await?;
    git(repo, &["config", "user.name", "AutoForge Demo"]).await?;

    // 创建目录结构
    mkdir(&format!("{repo}/src/components"))?;
    mkdir(&format!("{repo}/src/styles"))?;
    mkdir(&format!("{repo}/api"))?;
    mkdir(&format!("{repo}/frontend"))?;

    // 基础文件（main 分支初始提交）
    write(
        &format!("{repo}/src/components/Assistant.jsx"),
        r#"import React from 'react';

export default function Assistant({ items }) {
  return (
    <div className="assistant">
      {items.map(item => (
        <div key={item.id} className="item">{item.title}</div>
      ))}
    </div>
  );
}
"#,
    )?;

    write(
        &format!("{repo}/src/styles/assistant.css"),
        r#".assistant {
  padding: 16px;
}

.item {
  padding: 8px;
  border-bottom: 1px solid #eee;
}
"#,
    )?;

    write(
        &format!("{repo}/api/customers.py"),
        r#"def get_customers(page=1, limit=20):
    offset = (page - 1) * limit
    return db.query(
        'SELECT * FROM customers LIMIT ? OFFSET ?', limit, offset
    )
"#,
    )?;

    write(
        &format!("{repo}/export_service.py"),
        r#"def export_report(filters):
    data = db.query_large(filters)
    return generate_csv(data)
"#,
    )?;

    write(
        &format!("{repo}/api/exports.py"),
        r#"from export_service import export_report

def export_handler(request):
    return export_report(request.filters)
"#,
    )?;

    git(repo, &["add", "."]).await?;
    git(repo, &["commit", "-m", "chore: initial project setup"]).await?;

    // --- 2. 分支 autoforge/cr-001: 助手页标签筛选 ---
    git(repo, &["checkout", "-b", "autoforge/cr-001"]).await?;

    write(
        &format!("{repo}/src/components/Assistant.jsx"),
        r#"import React, { useState } from 'react';

export default function Assistant({ items }) {
  const [tag, setTag] = useState('all');
  const filtered =
    tag === 'all' ? items : items.filter(i => i.tag === tag);

  return (
    <div className="assistant">
      <div className="tag-bar">
        {['all', 'todo', 'done'].map(t => (
          <button
            key={t}
            className={'tag-btn' + (tag === t ? ' on' : '')}
            onClick={() => setTag(t)}
          >
            {t === 'all' ? '全部' : t === 'todo' ? '待办' : '已完成'}
          </button>
        ))}
      </div>
      {filtered.map(item => (
        <div key={item.id} className="item">{item.title}</div>
      ))}
    </div>
  );
}
"#,
    )?;

    write(
        &format!("{repo}/src/styles/assistant.css"),
        r#".assistant {
  padding: 16px;
}

.item {
  padding: 8px;
  border-bottom: 1px solid #eee;
}

.tag-bar {
  display: flex;
  gap: 8px;
  margin-bottom: 12px;
  padding: 6px 0;
  border-bottom: 1px solid var(--border);
}

.tag-btn {
  padding: 4px 12px;
  border-radius: 12px;
  border: 1px solid var(--border);
  background: transparent;
  cursor: pointer;
  font-size: 13px;
  transition: background 0.15s;
}

.tag-btn.on {
  background: var(--ember);
  color: white;
  border-color: var(--ember);
}
"#,
    )?;

    git(repo, &["add", "."]).await?;
    git(repo, &["commit", "-m", "feat: 助手页新增标签筛选栏"]).await?;
    git(repo, &["checkout", "main"]).await?;

    // --- 3. 分支 autoforge/cr-002: 导出改为异步 ---
    git(repo, &["checkout", "-b", "autoforge/cr-002"]).await?;

    write(
        &format!("{repo}/export_service.py"),
        r#"from celery import shared_task
import uuid, os

@shared_task
def export_report_async(filters, task_id):
    data = db.query_large(filters)
    out_dir = '/tmp/exports'
    os.makedirs(out_dir, exist_ok=True)
    csv_path = f'{out_dir}/{task_id}.csv'
    generate_csv(data, csv_path)
    notify_complete(task_id, csv_path)
    return csv_path

def start_export(filters):
    task_id = str(uuid.uuid4())
    export_report_async.delay(filters, task_id)
    return task_id
"#,
    )?;

    write(
        &format!("{repo}/api/exports.py"),
        r#"from export_service import start_export
from tasks import get_task_status

def export_handler(request):
    task_id = start_export(request.filters)
    return {'task_id': task_id, 'status': 'pending'}

def task_status_handler(request, task_id):
    status = get_task_status(task_id)
    if status == 'done':
        return {'status': 'done', 'download_url': f'/downloads/{task_id}.csv'}
    return {'status': status}
"#,
    )?;

    write(
        &format!("{repo}/frontend/Export.tsx"),
        r#"import React, { useState } from 'react';

export default function Export() {
  const [taskId, setTaskId] = useState<string | null>(null);
  const [status, setStatus] = useState('idle');
  const [url, setUrl] = useState<string | null>(null);

  const startExport = async () => {
    const res = await fetch('/api/exports', { method: 'POST' });
    const { task_id } = await res.json();
    setTaskId(task_id);
    setStatus('pending');
    pollStatus(task_id);
  };

  const pollStatus = (id: string) => {
    const iv = setInterval(async () => {
      const res = await fetch(`/api/exports/${id}/status`);
      const data = await res.json();
      setStatus(data.status);
      if (data.status === 'done') {
        clearInterval(iv);
        setUrl(data.download_url);
      }
    }, 2000);
  };

  return (
    <div className="export-panel">
      <button onClick={startExport} disabled={status === 'pending'}>
        {status === 'pending' ? '导出中…' : '导出报表'}
      </button>
      {url && <a href={url} download>下载文件</a>}
    </div>
  );
}
"#,
    )?;

    git(repo, &["add", "."]).await?;
    git(
        repo,
        &["commit", "-m", "feat: 导出接口改为 Celery 异步任务"],
    )
    .await?;
    git(repo, &["checkout", "main"]).await?;

    // --- 4. 分支 autoforge/cr-003: 分页修复 ---
    git(repo, &["checkout", "-b", "autoforge/cr-003"]).await?;

    write(
        &format!("{repo}/api/customers.py"),
        r#"def get_customers(cursor=None, limit=20):
    """cursor-based 分页，避免 offset 重复"""
    if cursor:
        return db.query(
            'SELECT * FROM customers WHERE id > ? ORDER BY id LIMIT ?',
            cursor, limit
        )
    return db.query(
        'SELECT * FROM customers ORDER BY id LIMIT ?', limit
    )
"#,
    )?;

    git(repo, &["add", "."]).await?;
    git(
        repo,
        &["commit", "-m", "fix: 分页改用 cursor-based 防止数据重复"],
    )
    .await?;
    git(repo, &["checkout", "main"]).await?;

    // --- 5. 创建 worktree 目录（让路径存在检查通过）---
    for dir in &["/tmp/wt/cr-001", "/tmp/wt/cr-002", "/tmp/wt/cr-003"] {
        mkdir(dir)?;
    }

    // --- 6. 确保前置数据存在（外键依赖链：project → issue → cr → worktree_session）---
    // projects
    for (id, name, slug, desc) in &[
        ("proj-vocant", "Vocant", "vocant", "AI 原生业务管理工具"),
        ("proj-another", "AnotherApp", "another-app", "内部协作平台"),
    ] {
        sqlx::query(
            "INSERT OR IGNORE INTO projects (id, name, slug, description, repo_path, branch_dev, branch_main, status)
             VALUES (?, ?, ?, ?, '', 'dev', 'main', 'active')",
        )
        .bind(id).bind(name).bind(slug).bind(desc)
        .execute(&state.db).await.map_err(|e| e.to_string())?;
    }

    // issues
    for (id, proj, title, cat) in &[
        ("issue-001", "proj-vocant", "助手页缺少标签筛选", "Feature"),
        ("issue-002", "proj-vocant", "导出报表接口超时 (>30s)", "Bug"),
        ("issue-003", "proj-another", "客户列表分页错乱", "Bug"),
    ] {
        sqlx::query(
            "INSERT OR IGNORE INTO issues (id, project_id, source_type, title, description, category, severity, priority, status, fingerprint)
             VALUES (?, ?, 'manual', ?, '', ?, 'high', 7, 'executing', ?)",
        )
        .bind(id).bind(proj).bind(title).bind(cat).bind(format!("fp-demo-{id}"))
        .execute(&state.db).await.map_err(|e| e.to_string())?;
    }

    // change_requests
    for (id, proj, issue, status) in &[
        ("cr-001", "proj-vocant", "issue-001", "pending_review_2"),
        ("cr-002", "proj-vocant", "issue-002", "pending_review_2"),
        ("cr-003", "proj-another", "issue-003", "merged"),
    ] {
        sqlx::query(
            "INSERT OR IGNORE INTO change_requests (id, project_id, issue_id, status, admin_id, target_branch)
             VALUES (?, ?, ?, ?, 'admin', 'dev')",
        )
        .bind(id).bind(proj).bind(issue).bind(status)
        .execute(&state.db).await.map_err(|e| e.to_string())?;
    }

    // worktree_sessions
    let report1 = "## 改动摘要\n为助手页新增标签筛选栏，支持全部/待办/已完成三种视图。\n\n## 修改文件列表\n- src/components/Assistant.jsx: 新增标签栏组件 (+34 -6)\n- src/styles/assistant.css: 标签栏样式 (+18)\n\n## 测试情况\n新增测试：3 个\n通过状态：全部通过 ✓\n覆盖率：81.3%（阈值 80%）\n\n## 潜在风险\n无破坏性变更。标签状态仅作用于前端视图。";
    let report2 = "## 改动摘要\n将导出接口改为 Celery 异步任务，前端轮询任务状态，完成后提供下载链接。\n\n## 修改文件列表\n- export_service.py: 重构为异步任务 (+67 -23)\n- api/exports.py: 新增 task_status 端点 (+18)\n- frontend/Export.tsx: 改为轮询模式 (+45 -12)\n\n## 测试情况\n新增测试：6 个\n通过状态：全部通过 ✓\n覆盖率：79.1%（低于阈值！）\n\n## 潜在风险\n覆盖率略低于阈值，建议补充边界测试。";
    let report3 = "## 改动摘要\n修复分页 offset 计算错误，改用 cursor-based 分页防止重复。\n\n## 修改文件列表\n- api/customers.py: 修改分页逻辑 (+8 -3)\n\n## 测试情况\n新增测试：2 个\n通过状态：全部通过 ✓\n覆盖率：83.7%\n\n## 潜在风险\n无。";

    for (id, cr, wt_path, branch, status, report) in &[
        (
            "ws-001",
            "cr-001",
            "/tmp/wt/cr-001",
            "autoforge/cr-001",
            "completed",
            report1,
        ),
        (
            "ws-002",
            "cr-002",
            "/tmp/wt/cr-002",
            "autoforge/cr-002",
            "completed",
            report2,
        ),
        (
            "ws-003",
            "cr-003",
            "/tmp/wt/cr-003",
            "autoforge/cr-003",
            "completed",
            report3,
        ),
    ] {
        sqlx::query(
            "INSERT OR IGNORE INTO worktree_sessions
             (id, change_request_id, worktree_path, branch_name, status, iteration_count, report_content, started_at, completed_at)
             VALUES (?, ?, ?, ?, ?, 1, ?, datetime('now','-2 hours'), datetime('now','-1 hour'))",
        )
        .bind(id).bind(cr).bind(wt_path).bind(branch).bind(status).bind(report)
        .execute(&state.db).await.map_err(|e| e.to_string())?;
    }

    // 更新项目 repo_path 为真实 git 仓库
    sqlx::query("UPDATE projects SET repo_path=? WHERE id IN ('proj-vocant','proj-another')")
        .bind(repo)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    // --- 7. 插入 preview_environments ---
    for (id, proj, session, url, status) in &[
        (
            "prev-001",
            "proj-vocant",
            "ws-001",
            "http://localhost:3001",
            "ready",
        ),
        (
            "prev-002",
            "proj-vocant",
            "ws-002",
            "http://localhost:3002",
            "ready",
        ),
        (
            "prev-003",
            "proj-another",
            "ws-003",
            "http://localhost:3003",
            "starting",
        ),
    ] {
        sqlx::query(
            "INSERT OR REPLACE INTO preview_environments
             (id, project_id, env_type, worktree_session_id, preview_url, status, created_at, ready_at)
             VALUES (?, ?, 'worktree', ?, ?, ?, datetime('now'), CASE WHEN ?='ready' THEN datetime('now') ELSE NULL END)",
        )
        .bind(id)
        .bind(proj)
        .bind(session)
        .bind(url)
        .bind(status)
        .bind(status)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(format!(
        "演示 git 仓库已创建于 {repo}，preview 环境数据已填充"
    ))
}
