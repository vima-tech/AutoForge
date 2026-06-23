# AutoForge — SQLite 容量规划

**版本：** v1.0
**日期：** 2026-06-03
**状态：** 实施参考（与 `autoforge-design.md` 配套）
**评估基线：** 单桌面实例 · 10+ 项目 · 10–100 写/小时 · 自托管

---

## 1. 评估结论与适用边界

### 1.1 一句话结论

**在自托管单桌面实例场景下，SQLite 足以支撑 10+ 项目、5 年内、10–100 写/小时的生产负载，**且有 1–2 个数量级的性能余量。无需迁移到 PostgreSQL，但需要执行本文档定义的 P0/P1/P2 改造以释放性能并建立长期可维护性。

### 1.2 适用边界

此结论成立的前提（任一不成立需重新评估）：

| 前提 | 当前设计中的位置 |
|------|------------------|
| 单桌面实例、单用户、单 DB 文件 | `lib.rs:28` DB 路径固定为 `<app_data_dir>/autoforge.db` |
| 自托管，无云端同步 | `autoforge-design.md` §17 决策表 |
| 写吞吐 ≤ 100 条/小时 | 见第 4 章体量推算 |
| 单 DB < 50 GB | 见第 4 章 5 年推算（~5.8 GB） |
| 读并发 < 10 | 设计 §11.2 默认 `max_slots=5` |
| 无跨实例数据合并需求 | 自托管形态天然排除 |

### 1.3 必须迁移到 PostgreSQL 的触发条件

出现**任一**情况时，**应启动** 6 个月内迁移计划：

- 单实例 DB > 50 GB（备份窗口失控、VACUUM 慢）
- 写吞吐持续 > 50 TPS（接近 SQLite 临界区）
- 引入多用户共享一个工厂（违反自托管定位）
- 需要跨设备审核同步 / 移动端访问
- 需要主从复制 / 高可用

迁移成本预估：**2–3 周**（sqlx 已是 PostgreSQL-ready，切换 `Cargo.toml` feature flag 即可，详见第 8 章）。

---

## 2. 容量基线假设

### 2.1 业务假设（10+ 项目，10–100 写/小时中场景）

| 维度 | 假设值 | 来源 |
|------|--------|------|
| 注册项目数 | 10+（活跃 6+，其余 paused） | 评估基线 |
| 每项目日均 Issue | 5–20 条（活跃项目） | 设计 §15 效率指标推导 |
| 反馈来源分布 | Widget 60% · Admin 20% · GitHub 15% · Monitor 5% | 设计 §6.1 |
| Issue → CR 转化率 | ~85%（设计 §10.1 期望 60% 首次通过 + 25% 迭代） | 设计 §15 |
| CR 平均迭代轮数 | 1.5（首次通过 60% + 二次通过 25% + 三次 10% + ≥4 次 5%） | 设计 §10.4 软上限 |
| Worktree 平均存活 | 30 分钟（代码 + 测试 + 预览） | 设计 §10.4 |
| Widget 消息保留 | 180 天 | 设计 §6.1 隐私策略 |
| Job executions 保留 | 30 天（软治理） | 本文 §6.2 |
| 管理员审核节奏 | < 15 分钟/条 | 设计 §15 指标 |

### 2.2 写入频率分解

按 50 条/小时中位场景拆解：

| 写入类型 | 频率（条/小时） | 占比 | 单次写入语句数 |
|---------|----------------|------|--------------|
| Issue 提交 | ~15 | 30% | 3（去重查 + INSERT + 派发 enqueue） |
| 分析任务完成 | ~15 | 30% | 2（INSERT analysis + UPDATE issue） |
| 审核节点 1 通过 | ~13 | 25% | 3（INSERT CR + UPDATE issue + enqueue） |
| 审核节点 2 通过 | ~13 | 25% | 2（UPDATE CR + enqueue merge） |
| 合并完成 | ~12 | 24% | 3（UPDATE CR + issue + worktree） |
| Worktree 状态更新 | ~25 | 50% | 1–2 |
| 消息 / 巡检发现 | < 5 | 10% | 1–2 |

**峰值写 TPS ≈ 0.5–1.5**（按最坏秒级突发估算），SQLite 极限 ~1000+ TPS，**余量 3 个数量级**。

### 2.3 读频率分解

| 读类型 | 频率 | 缓存收益 |
|--------|------|---------|
| `list_projects` | 每次进入 Projects 页（< 10/小时） | 全表 ~10 行，可常驻 |
| `list_issues` | Dashboard 轮询（10–30/小时） | 索引覆盖 |
| `list_change_requests` | Audit 轮询（10–30/小时） | 索引覆盖 |
| `list_conversations` | 进入会话页（5/小时） | 当前 N+1（需修） |
| `pipeline_stats` | 前端 5s 轮询（720/小时） | 当前 11 次独立查询（需修） |
| `get_change_request` / `get_issue` | 单条查询 | 高频索引命中 |

---

## 3. 当前实现盘点

### 3.1 DB 初始化（`src-tauri/src/db.rs:7-19`）

```rust
pub async fn init(db_path: &str) -> Result<Db> {
    let parent = Path::new(db_path).parent().unwrap_or(Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let url = format!("sqlite://{}?mode=rwc", db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)              // ⚠ 偏高，SQLite 1 writer + N readers
        .connect(&url)
        .await?;
    sqlx::query("PRAGMA journal_mode=WAL").execute(&pool).await?;
    sqlx::query("PRAGMA foreign_keys=ON").execute(&pool).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
```

**问题清单：**

| 缺失项 | 影响 | 修复优先级 |
|--------|------|----------|
| `PRAGMA synchronous` | WAL 下默认 FULL，写慢 2–3× | P0 |
| `PRAGMA busy_timeout` | 写锁竞争时返回 SQLITE_BUSY | P0 |
| `PRAGMA cache_size` | 读缓存默认 2MB 太小 | P0 |
| `PRAGMA temp_store` | 排序/临时表走磁盘 | P1 |
| `PRAGMA mmap_size` | 无法利用内存映射读 | P1 |
| `max_connections(8)` | 高并发写时锁竞争 | P0 |
| WAL autocheckpoint | 默认 1000 页，OK | 无需改 |

### 3.2 索引现状（`migrations/0001_initial.sql`）

| 表 | 已有索引 | 缺失关键索引 |
|----|---------|------------|
| `issues` | `(project_id)`, `(status)` | `(fingerprint)` 去重查询全表扫 |
| `change_requests` | `(project_id)`, `(status)` | `(issue_id)` 反查 |
| `worktree_sessions` | **无** | `(change_request_id)` **每次合并/查询都全表扫** |
| `conversations` | **无** | `(created_at)` ORDER BY 走临时排序 |
| `projects` | 仅 PK + UNIQUE(slug) | `(created_at)` ORDER BY 走临时排序 |
| `messages` | `(conversation_id, created_at)` ✓ | OK |
| `job_executions` | `(status)`, UNIQUE(idempotency_key) | `(enqueued_at)` 清理/排序 |

### 3.3 事务现状

**全代码库 0 处事务使用**（`db.begin()` 计数 = 0）。以下多语句操作存在部分失败风险：

| 位置 | 操作 | 风险 |
|------|------|------|
| `commands/issues.rs:54-136` `submit_issue` | SELECT 去重 → INSERT → 另一次 SELECT → enqueue | 去重与插入之间有竞态 |
| `commands/change_requests.rs:146-224` `review_1` | INSERT CR + UPDATE issue + enqueue | 中间崩溃 → 状态机分裂 |
| `commands/change_requests.rs:228-310` `review_2` | UPDATE CR + enqueue | 同上 |
| `tasks/runner.rs:79-115` `enqueue` | INSERT OR IGNORE + 单独 SELECT | writer lock 持有 2× |
| `tasks/execution.rs:8-167` | 4 SELECT + 5 UPDATE 串行 | 中间崩溃 → worktree 记录与 CR 状态不一致 |

### 3.4 写热点（按调用频率）

| 路径 | 频率 | 现状 |
|------|------|------|
| `pipeline_stats`（`system.rs:71-178`） | 5s 轮询 = 720/小时 | 11 条独立 COUNT 查询 |
| `list_conversations`（`conversations.rs:8-61`） | 每次打开页 | 1 + 2N 查询（N+1） |
| `submit_issue`（`issues.rs:54-136`） | 15/小时 | 3 次往返，无事务 |
| `enqueue`（`runner.rs:79-115`） | 80/小时 | 2 次往返，无事务 |
| `get_code_diff`（`change_requests.rs:83-142`） | 每次审核 | 3 条串行查询 |

---

## 4. 体量推算表

### 4.1 行数与体积推算（5 年）

按 50 条/小时中位场景，假设 10 个活跃项目、5 年连续运行：

| 表 | 年行数 | 单行平均 | 1 年 | 3 年 | 5 年 |
|----|-------|---------|------|------|------|
| `projects` | +3 | 0.3 KB | 13 行 | 19 行 | 25 行 |
| `issues` | ~30,000 | 2 KB | 60 MB | 180 MB | 300 MB |
| `issue_analyses`（含 raw_llm_output） | ~30,000 | 5 KB | 150 MB | 450 MB | 750 MB |
| `change_requests` | ~25,000 | 0.5 KB | 13 MB | 38 MB | 63 MB |
| `worktree_sessions`（含 prompt + report） | ~45,000 | 8 KB | 360 MB | 1.1 GB | 1.8 GB |
| `messages`（180 天滚动） | ~50,000 | 3 KB | 50 MB（峰值） | 50 MB | 50 MB |
| `job_executions`（30 天滚动） | ~120,000 | 1 KB | 10 MB（峰值） | 10 MB | 10 MB |
| **DB 总量（含索引/WAL 开销 ~30%）** | | | **~840 MB** | **~2.5 GB** | **~4.2 GB** |

> 体积估算含 WAL 日志 + 索引开销。`messages` 和 `job_executions` 在软治理后保持滚动窗口稳定。

### 4.2 写吞吐推算

| 场景 | 平均写/小时 | 峰值写/秒 | 与 SQLite 极限比 |
|------|------------|----------|----------------|
| 轻（设计基线） | 10 | 0.05 | 1/20,000 |
| 中（评估基线） | 50 | 0.2 | 1/5,000 |
| 重（Widget 活跃 + 高频巡检） | 100 | 0.5 | 1/2,000 |
| 理论极限 | 100,000+ | ~30 | 1/1 |

### 4.3 备份窗口推算

| DB 大小 | `sqlite3 .backup` 耗时 | `rsync` 增量 | 备注 |
|--------|------------------------|-------------|------|
| 1 GB | 2–5s | < 1s | 桌面级，无感 |
| 5 GB | 10–25s | 2–3s | 需停机或 WAL 模式备份 |
| 10 GB | 30–60s | 5–10s | 需考虑 litestream |
| 50 GB | 2–5 min | 30s+ | **触发迁移评估** |

---

## 5. 改造路线图

### P0 — 立即修（1 个 sprint，~1 天）

#### 5.1 P0.1 新增 `migrations/0004_indexes_and_pragma.sql`

```sql
-- 0004_indexes_and_pragma.sql
-- 补全缺失的关键索引

CREATE INDEX IF NOT EXISTS ix_wt_cr        ON worktree_sessions(change_request_id);
CREATE INDEX IF NOT EXISTS ix_issues_fp    ON issues(fingerprint);
CREATE INDEX IF NOT EXISTS ix_cr_issue     ON change_requests(issue_id);
CREATE INDEX IF NOT EXISTS ix_conv_created ON conversations(created_at DESC);
CREATE INDEX IF NOT EXISTS ix_proj_created ON projects(created_at DESC);
CREATE INDEX IF NOT EXISTS ix_job_enqueued ON job_executions(enqueued_at DESC);
```

#### 5.2 P0.2 重写 `db.rs`

```rust
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::path::Path;
use anyhow::Result;

pub type Db = SqlitePool;

pub async fn init(db_path: &str) -> Result<Db> {
    let parent = Path::new(db_path).parent().unwrap_or(Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let url = format!("sqlite://{}?mode=rwc", db_path);

    // 连接池：SQLite 1 writer + N readers，4 个连接足够覆盖
    // 5 并发 Claude Code + 前端偶尔查，实际并发读 3、写 1
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await?;

    // PRAGMA 必须在每个连接上生效
    sqlx::query("PRAGMA journal_mode=WAL").execute(&pool).await?;
    sqlx::query("PRAGMA foreign_keys=ON").execute(&pool).await?;
    sqlx::query("PRAGMA synchronous=NORMAL").execute(&pool).await?;  // WAL 下安全
    sqlx::query("PRAGMA busy_timeout=5000").execute(&pool).await?;   // 5s 等待
    sqlx::query("PRAGMA cache_size=-64000").execute(&pool).await?;  // 64MB
    sqlx::query("PRAGMA temp_store=MEMORY").execute(&pool).await?;  // 临时表走内存
    sqlx::query("PRAGMA mmap_size=268435456").execute(&pool).await?; // 256MB 内存映射读
    sqlx::query("PRAGMA wal_autocheckpoint=1000").execute(&pool).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
```

#### 5.3 P0.3 事务化 `enqueue`（`tasks/runner.rs:79-115`）

```rust
pub async fn enqueue(
    db: &Db,
    tx: &JobSender,
    job_type: &str,
    idempotency_key: &str,
    payload: JobPayload,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let payload_json = serde_json::to_string(&payload)?;

    let mut db_tx = db.begin().await?;

    sqlx::query(
        "INSERT OR IGNORE INTO job_executions (id, idempotency_key, job_type, payload, status)
         VALUES (?, ?, ?, ?, 'pending')"
    )
    .bind(&id)
    .bind(idempotency_key)
    .bind(job_type)
    .bind(&payload_json)
    .execute(&mut *db_tx)
    .await?;

    let (actual_id,): (String,) = sqlx::query_as(
        "SELECT id FROM job_executions WHERE idempotency_key=?"
    )
    .bind(idempotency_key)
    .fetch_one(&mut *db_tx)
    .await?;

    db_tx.commit().await?;

    let _ = tx.send(JobMsg {
        job_id: actual_id.clone(),
        payload,
    }).await;

    Ok(actual_id)
}
```

### P1 — 下一个 sprint（2–3 天）

#### 5.4 P1.1 事务化 `review_1`（`commands/change_requests.rs:146-224`）

```rust
#[tauri::command]
pub async fn review_1(
    issue_id: String,
    decision: Review1Decision,
    state: State<'_, AppState>,
) -> Result<ChangeRequest, String> {
    if decision.decision == "approved" {
        let issue = sqlx::query_as::<_, crate::models::issue::Issue>(
            "SELECT * FROM issues WHERE id=?"
        )
        .bind(&issue_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        let project = sqlx::query_as::<_, crate::models::project::Project>(
            "SELECT * FROM projects WHERE id=?"
        )
        .bind(&issue.project_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

        let cr_id = Uuid::new_v4().to_string();
        let admin_id = decision.admin_id.unwrap_or_else(|| "admin".to_string());

        // 事务化：CR 插入 + Issue 状态更新
        let mut tx = state.db.begin().await.map_err(|e| e.to_string())?;

        sqlx::query(
            "INSERT INTO change_requests
             (id, project_id, issue_id, status, admin_id, admin_suggestions_1, target_branch)
             VALUES (?, ?, ?, 'pending_execution', ?, ?, ?)"
        )
        .bind(&cr_id)
        .bind(&issue.project_id)
        .bind(&issue_id)
        .bind(&admin_id)
        .bind(decision.suggestions.as_deref().unwrap_or(""))
        .bind(&project.branch_dev)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query("UPDATE issues SET status='pending_execution', updated_at=datetime('now') WHERE id=?")
            .bind(&issue_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        tx.commit().await.map_err(|e| e.to_string())?;

        // enqueue 在事务外执行（它自己有事务）
        let idem_key = format!("execution:{}", cr_id);
        let _ = enqueue(
            &state.db,
            &state.job_tx,
            "execution",
            &idem_key,
            JobPayload::Execution {
                change_request_id: cr_id.clone(),
                project_id: issue.project_id.clone(),
            },
        )
        .await;

        sqlx::query_as::<_, ChangeRequest>("SELECT * FROM change_requests WHERE id=?")
            .bind(&cr_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())
    } else {
        sqlx::query("UPDATE issues SET status='rejected', updated_at=datetime('now') WHERE id=?")
            .bind(&issue_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
        Err("Issue rejected".to_string())
    }
}
```

#### 5.5 P1.2 消除 `list_conversations` 的 N+1（`commands/conversations.rs:8-61`）

```rust
#[tauri::command]
pub async fn list_conversations(
    state: State<'_, AppState>,
) -> Result<Vec<ConversationDetail>, String> {
    // 单次查询：会话 + 成员聚合 + 最后一条消息
    let rows = sqlx::query_as::<_, (String, String, Option<String>, String, String, String, String)>(
        r#"
        SELECT
            c.id, c.type, c.name, c.color, c.initial, c.created_at,
            COALESCE(
                (SELECT GROUP_CONCAT(agent_id)
                 FROM conversation_members
                 WHERE conversation_id = c.id),
                ''
            ) AS members_csv,
            COALESCE(
                (SELECT content_json
                 FROM messages
                 WHERE conversation_id = c.id
                 ORDER BY created_at DESC
                 LIMIT 1),
                ''
            ) AS last_message,
            COALESCE(
                (SELECT created_at
                 FROM messages
                 WHERE conversation_id = c.id
                 ORDER BY created_at DESC
                 LIMIT 1),
                ''
            ) AS last_time
        FROM conversations c
        ORDER BY c.created_at DESC
        "#
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let details = rows
        .into_iter()
        .map(|(id, conv_type, name, color, initial, created_at, members_csv, last_message, last_time)| {
            let members: Vec<String> = if members_csv.is_empty() {
                vec![]
            } else {
                members_csv.split(',').map(|s| s.to_string()).collect()
            };
            let (last_msg_opt, last_time_opt) = if last_message.is_empty() {
                (None, None)
            } else {
                (Some(last_message), Some(last_time))
            };
            ConversationDetail {
                id,
                conv_type,
                name,
                color,
                initial,
                created_at,
                members,
                unread: 0,
                last_message: last_msg_opt,
                last_time: last_time_opt,
            }
        })
        .collect();

    Ok(details)
}
```

#### 5.6 P1.3 合并 `pipeline_stats` 的 11 条 COUNT（`commands/system.rs:71-178`）

```rust
let stats = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64, i64, String)>(
    r#"
    SELECT
        SUM(CASE WHEN status='pending_analysis' THEN 1 ELSE 0 END),
        SUM(CASE WHEN status='pending_review_1' THEN 1 ELSE 0 END),
        SUM(CASE WHEN status='executing' THEN 1 ELSE 0 END),
        SUM(CASE WHEN status='pending_review_2' THEN 1 ELSE 0 END),
        SUM(CASE WHEN status='merged' THEN 1 ELSE 0 END),
        SUM(CASE WHEN status='rejected' THEN 1 ELSE 0 END),
        COUNT(*),
        (SELECT COUNT(*) FROM projects WHERE status='active'),
        GROUP_CONCAT(CASE WHEN status='executing' THEN id END)
    FROM issues
    "#
)
.fetch_one(&state.db)
.await
.map_err(|e| e.to_string())?;
```

### P2 — 软治理（2–3 天）

详见第 6 章。

### P3 — 长期（1+ 月）

FTS5 接入、PostgreSQL 迁移预案、备份/恢复流程。详见第 7、8 章。

---

## 6. 软治理方案

### 6.1 治理目标

让 DB 体积稳定在 5 GB 以下，避免 `messages` / `job_executions` 无界增长。

### 6.2 保留策略

| 表 | 保留期 | 治理动作 | 原因 |
|----|--------|---------|------|
| `messages` | 180 天 | 归档到 `messages_archive` 后从主表删除 | 设计 §6.1 Widget 隐私策略 |
| `job_executions` | 30 天 | 直接删除（无需归档） | 任务执行流水，审计价值低 |
| `worktree_sessions` | 永久 | 仅清理 status='failed' 的孤儿 | 设计记录 |
| `issue_analyses` | 永久 | `raw_llm_output` 可选压缩 | 高审计价值 |
| `change_requests` | 永久 | 无清理 | 核心审计链 |
| `admin_decisions` | 永久 | 无清理 | 核心审计链 |

### 6.3 归档表 Schema

新增 `migrations/0005_archive_tables.sql`：

```sql
-- 消息归档表（结构与 messages 一致，额外加 archived_at）
CREATE TABLE IF NOT EXISTS messages_archive (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    from_agent      TEXT,
    content_json    TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    archived_at     TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_msg_arch_conv ON messages_archive(conversation_id, created_at);
CREATE INDEX IF NOT EXISTS ix_msg_arch_time ON messages_archive(archived_at);

-- 会话归档表（按月分区？SQLite 无原生分区，用应用层判断）
-- 此处用 archived_at 索引近似
CREATE TABLE IF NOT EXISTS conversations_archive (
    id         TEXT PRIMARY KEY,
    type       TEXT NOT NULL,
    name       TEXT,
    color      TEXT NOT NULL,
    initial    TEXT,
    created_at TEXT NOT NULL,
    archived_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- job_executions 无需归档表（直接删除）
```

### 6.4 清理任务实现

新建 `src-tauri/src/tasks/maintenance.rs`：

```rust
use crate::db::Db;
use chrono::{Duration, Utc};
use std::time::Duration as StdDuration;
use tracing::{info, warn};
use tokio::time::interval;

pub fn spawn(db: Db) {
    tokio::spawn(async move {
        let mut ticker = interval(StdDuration::from_secs(24 * 3600)); // 每 24h
        ticker.tick().await; // 启动时立即跑一次

        loop {
            ticker.tick().await;
            if let Err(e) = run_once(&db).await {
                warn!("maintenance task failed: {}", e);
            }
        }
    });
}

async fn run_once(db: &Db) -> anyhow::Result<()> {
    info!("maintenance task started");

    // 1. 归档 180 天前的消息
    let cutoff_msg = (Utc::now() - Duration::days(180)).to_rfc3339();
    let archived = sqlx::query(
        r#"
        INSERT OR IGNORE INTO messages_archive
            (id, conversation_id, from_agent, content_json, created_at)
        SELECT id, conversation_id, from_agent, content_json, created_at
        FROM messages
        WHERE created_at < ?
        "#
    )
    .bind(&cutoff_msg)
    .execute(db)
    .await?
    .rows_affected();

    let deleted_msg = sqlx::query("DELETE FROM messages WHERE created_at < ?")
        .bind(&cutoff_msg)
        .execute(db)
        .await?
        .rows_affected();

    info!("messages: archived={}, deleted={}", archived, deleted_msg);

    // 2. 清理 30 天前的 job_executions
    let cutoff_job = (Utc::now() - Duration::days(30)).to_rfc3339();
    let deleted_jobs = sqlx::query("DELETE FROM job_executions WHERE enqueued_at < ?")
        .bind(&cutoff_job)
        .execute(db)
        .await?
        .rows_affected();

    info!("job_executions: deleted={}", deleted_jobs);

    // 3. 清理失败的孤儿 worktree
    let orphan_wt = sqlx::query(
        "DELETE FROM worktree_sessions
         WHERE status='failed'
         AND completed_at < ?"
    )
    .bind(&cutoff_job)
    .execute(db)
    .await?
    .rows_affected();

    info!("worktree_sessions: orphan_deleted={}", orphan_wt);

    // 4. WAL checkpoint (TRUNCATE 模式回收空间)
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(db).await?;

    // 5. 完整性检查
    let (integrity,): (String,) = sqlx::query_as("PRAGMA integrity_check")
        .fetch_one(db)
        .await?;
    if integrity != "ok" {
        warn!("integrity check failed: {}", integrity);
    }

    info!("maintenance task completed");
    Ok(())
}
```

在 `lib.rs:32-34` setup 中挂载：

```rust
let db = tauri::async_runtime::block_on(async {
    db::init(&db_path).await.expect("db init failed")
});

// 启动后台维护任务
tasks::maintenance::spawn(db.clone());
```

### 6.5 归档数据访问

归档表与主表结构一致，但前端不直接查询。审计场景下通过管理命令访问：

```rust
#[tauri::command]
pub async fn list_archived_messages(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::conversation::Message>, String> {
    sqlx::query_as::<_, crate::models::conversation::Message>(
        "SELECT * FROM messages_archive
         WHERE conversation_id=?
         ORDER BY created_at DESC
         LIMIT 500"
    )
    .bind(&conversation_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())
}
```

---

## 7. 监控指标与基线采集

### 7.1 关键指标（启动时一次性采集 + 周度采样）

| 指标 | 采集方式 | 阈值 |
|------|---------|------|
| DB 文件大小 | `std::fs::metadata(db_path).len()` | < 5 GB 正常，> 10 GB 告警 |
| WAL 文件大小 | `db_path-wal` 文件大小 | < 100 MB 正常 |
| 各表行数 | `SELECT COUNT(*)` from 7 张表 | 见 §4.1 |
| 索引碎片率 | `PRAGMA index_info` + `freelist_count` | < 20% 正常 |
| 写锁等待次数 | `PRAGMA busy_timeout` 触发次数（需埋点） | < 1/小时正常 |
| 查询 P99 延迟 | sqlx middleware 计时 | < 50ms 正常 |

### 7.2 启动报告实现

新增 `commands/system.rs` 中的 `db_health`：

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct DbHealth {
    pub db_size_bytes: u64,
    pub wal_size_bytes: u64,
    pub table_counts: std::collections::HashMap<String, i64>,
    pub index_health: String,
    pub last_vacuum: Option<String>,
    pub last_checkpoint: Option<String>,
}

#[tauri::command]
pub async fn db_health(state: State<'_, AppState>) -> Result<DbHealth, String> {
    let db_path = crate::state::db_path();
    let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
    let wal_size = std::fs::metadata(format!("{}-wal", db_path))
        .map(|m| m.len()).unwrap_or(0);

    let mut counts = std::collections::HashMap::new();
    for table in &["projects", "issues", "issue_analyses", "change_requests",
                   "worktree_sessions", "messages", "job_executions"] {
        let (c,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {}", table))
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;
        counts.insert(table.to_string(), c);
    }

    let (integrity,): (String,) = sqlx::query_as("PRAGMA integrity_check")
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    Ok(DbHealth {
        db_size_bytes: db_size,
        wal_size_bytes: wal_size,
        table_counts: counts,
        index_health: integrity,
        last_vacuum: None,
        last_checkpoint: None,
    })
}
```

### 7.3 基线采集时机

- **M11 端到端验证启动时**：采集 1 次，作为后续对比基线
- **每周末**：自动采集并写入 `metrics_history` 表
- **DB 大小 > 1 GB 阈值时**：自动触发采集并输出到 Review Portal

### 7.4 告警阈值

| 指标 | 黄色（关注） | 红色（行动） |
|------|-------------|------------|
| DB 大小 | > 3 GB | > 8 GB |
| WAL 大小 | > 200 MB | > 1 GB |
| `messages` 行数 | > 30k | > 100k |
| `job_executions` 行数 | > 50k | > 200k |
| integrity_check | warn | fail |
| 查询 P99 延迟 | > 100ms | > 500ms |

---

## 8. SQLite 容量边界与 PostgreSQL 迁移

### 8.1 SQLite 实测极限

| 维度 | 极限 | AutoForge 5 年推算 | 余量 |
|------|------|-------------------|------|
| DB 大小 | 281 TB | 4.2 GB | 65,000× |
| 行数/表 | 2^64 | 150k | 不可达 |
| 写吞吐 | ~1k TPS | 0.5 TPS 峰值 | 2,000× |
| 单 writer | 1 | 1 | 满载 |

### 8.2 触发迁移评估的硬指标

满足**任一**条件时启动迁移评估（不需要立刻迁移，但需立项）：

- DB > 50 GB
- 写 TPS 持续 > 50
- 引入多用户共享
- 备份窗口 > 5 分钟
- VACUUM > 10 秒

### 8.3 迁移成本估算

迁移路径：`sqlx + SQLite` → `sqlx + PostgreSQL`

| 工作项 | 估时 | 说明 |
|--------|------|------|
| `Cargo.toml` feature 切换 | 0.5h | `sqlite` → `postgres` |
| SQL 方言差异修复 | 1–2 天 | `datetime('now')` → `NOW()`、`AUTOINCREMENT` → `SERIAL` 等 |
| PRAGMA 移除 | 0.5h | 部分 PRAGMA 无 PG 对应 |
| 索引重建 | 1h | 部分 SQLite 优化语法不适用 |
| 迁移脚本编写 | 2–3 天 | 7 张表 + 索引 + 归档表 |
| 集成测试 | 2–3 天 | 用 `sqlx::test` 套件 |
| 回退预案 | 1 天 | 双写 + 影子读 |
| **总计** | **2–3 周** | 含回归测试 |

### 8.4 迁移前置条件清单

- [ ] DB 大小触发阈值
- [ ] 立项 ADR 文档（决策记录）
- [ ] PostgreSQL 部署（建议 14+，自托管用 `postgres:14-alpine` 容器）
- [ ] 迁移脚本（`sqlx migrate add` 转换）
- [ ] 双写窗口（最少 1 周影子对比）
- [ ] 回退 SOP

---

## 9. 验收标准

### 9.1 P0 改造验收

- [ ] 所有新增索引通过 `EXPLAIN QUERY PLAN` 验证为索引覆盖（非 SCAN）
- [ ] `db.rs` 5 个 PRAGMA 全部生效（启动日志可查）
- [ ] `enqueue` 函数用 `db.begin()` / `db_tx.commit()` 包裹
- [ ] `cargo test` 全部通过
- [ ] `cargo clippy -- -D warnings` 全部通过

### 9.2 P1 改造验收

- [ ] `review_1` / `review_2` / `submit_issue` / `tasks/execution` 全部事务化
- [ ] `list_conversations` 1 次查询（不再 N+1）
- [ ] `pipeline_stats` ≤ 3 次查询（原 11 次）
- [ ] 前后端联调：Dashboard 加载 < 200ms（含网络往返）
- [ ] 5 个并发 CR 跑通压测脚本（见 §9.5）

### 9.3 P2 软治理验收

- [ ] `messages_archive` / 清理任务已部署
- [ ] 模拟运行 1 周后 `messages` 数量稳定在 180 天滚动窗口
- [ ] `job_executions` 数量稳定在 30 天滚动窗口
- [ ] 维护任务失败有日志告警

### 9.4 P3 长期验收

- [ ] DB 启动报告可读（Review Portal 显示）
- [ ] 7 项监控指标采集
- [ ] PostgreSQL 迁移 checklist 已 review
- [ ] 备份/恢复 SOP 已文档化

### 9.5 压测脚本骨架

`scripts/bench_capacity.rs`：

```rust
//! 容量压测脚本：模拟 10 项目 × 50 写/小时
//! 运行：cargo run --release --bin bench-capacity

use autoforge_lib::db;
use sqlx::Row;
use std::time::Instant;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let db = db::init("/tmp/autoforge-bench.db").await?;

    // 1. 准备：插入 10 个项目
    for i in 0..10 {
        sqlx::query("INSERT INTO projects (id, name, slug) VALUES (?, ?, ?)")
            .bind(format!("bench-proj-{}", i))
            .bind(format!("BenchProj{}", i))
            .bind(format!("bench-{}", i))
            .execute(&db).await?;
    }

    // 2. 写压测：批量插入 issues，模拟 50 写/小时
    let start = Instant::now();
    let n = 500;
    for i in 0..n {
        let pid = format!("bench-proj-{}", i % 10);
        sqlx::query(
            "INSERT INTO issues (id, project_id, source_type, title, description, fingerprint)
             VALUES (?, ?, 'manual', ?, ?, ?)"
        )
        .bind(format!("bench-iss-{}", i))
        .bind(&pid)
        .bind(format!("Bench issue {}", i))
        .bind(format!("Description for bench issue {}", i))
        .bind(format!("fp-{}", i))
        .execute(&db).await?;
    }
    let write_elapsed = start.elapsed();
    println!("Wrote {} issues in {:?}", n, write_elapsed);
    println!("Per-insert: {:?}", write_elapsed / n);

    // 3. 读压测：list_issues 模式
    let start = Instant::now();
    for _ in 0..100 {
        let _: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM issues WHERE project_id=? ORDER BY created_at DESC LIMIT 100"
        )
        .bind("bench-proj-0")
        .fetch_all(&db).await?;
    }
    let read_elapsed = start.elapsed();
    println!("100 list_issues in {:?}", read_elapsed);

    // 4. DB 大小
    let size = std::fs::metadata("/tmp/autoforge-bench.db")?.len();
    println!("DB size: {:.2} MB", size as f64 / 1024.0 / 1024.0);

    Ok(())
}
```

**验收阈值**（在 M11 真实数据上跑）：

- 单次 INSERT（含去重查）P99 < 10ms
- `list_issues` P99 < 50ms
- 100 次连续 `pipeline_stats` < 500ms
- 5 并发 worktree 跑 1 小时无 SQLITE_BUSY

---

## 附录 A：实施 Checklist

按 P0 → P3 顺序勾选：

**P0（1 天）**
- [ ] 新增 `migrations/0004_indexes_and_pragma.sql`
- [ ] 重写 `src-tauri/src/db.rs`（PRAGMA + pool）
- [ ] 事务化 `tasks/runner.rs::enqueue`
- [ ] `cargo test` 通过
- [ ] 启动日志确认 PRAGMA 生效

**P1（2-3 天）**
- [ ] 事务化 `submit_issue` / `review_1` / `review_2`
- [ ] 事务化 `tasks/execution.rs`
- [ ] 优化 `list_conversations`（消除 N+1）
- [ ] 优化 `pipeline_stats`（合并 COUNT）
- [ ] 跑 `scripts/bench_capacity.rs`，记录基线

**P2（2-3 天）**
- [ ] 新增 `migrations/0005_archive_tables.sql`
- [ ] 新增 `tasks/maintenance.rs`
- [ ] 在 `lib.rs` 挂载维护任务
- [ ] 新增 `list_archived_messages` 命令
- [ ] 模拟运行 1 周验证

**P3（持续）**
- [ ] 新增 `db_health` 命令
- [ ] Review Portal 集成 DB 监控面板
- [ ] ADR 文档：迁移到 PostgreSQL 的触发条件
- [ ] 备份/恢复 SOP 文档

---

## 附录 B：相关文档引用

- `docs/autoforge-design.md` §6.1 Widget 隐私策略（消息保留 180 天）
- `docs/autoforge-design.md` §11.2 并发控制（`max_slots=5`）
- `docs/autoforge-design.md` §12 数据模型（admin_decisions 等未建表项）
- `docs/autoforge-design.md` §15 成功指标（驱动软治理优先级）

---

**文档结束**
