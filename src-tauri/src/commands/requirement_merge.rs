//! 同文件多需求合并 —— 候选探测。
//!
//! 在「需求审核」队列里找出**命中同一文件**、可合并成一次编码执行的需求组。
//! 触发信号取自需求分析产出 `issue_analyses.analysis_json`（`scope.affected_files` ∪
//! `claude_code_brief.files_to_touch`）。两个关键护栏：
//!  - **枢纽文件降权**：lib.rs / mod.rs / index.ts / migrations / *.md 等几乎人人命中的
//!    登记类文件，以及在当前队列里高频出现的文件，不计入「实质重叠」，避免虚假大簇。
//!  - **clique 语义**：一个候选组要求**全体成员共享同一实质文件**，而非两两链式串联。
//!
//! 探测纯规则、零 LLM 成本。核心逻辑放在纯函数 `compute_candidates` 便于单测；命令层只做
//! 「取 state → 查库取 spec → 调纯函数」。

use crate::models::change_request::MergeCandidate;
use crate::state::AppState;
use std::collections::{BTreeSet, HashMap};
use tauri::State;

/// 组内需求数上限：超过视为枢纽未滤净，跳过该组（防滚雪球）。
const MAX_GROUP: usize = 5;
/// 合并后去重总文件数上限：超过仍返回但标记 weak（blast radius 过大，收益存疑）。
const MAX_BLAST: usize = 15;
/// 频率枢纽阈值：某文件被 ≥ 该条数的需求命中，且占比 ≥ 50%，视为枢纽。
const HUB_MIN_COUNT: usize = 4;
/// 重叠占比下限（设计 §2.4）：共享实质文件数占「最小成员文件集」比例低于此值 → 弱候选。
/// 占比低说明各需求主要动各自私有文件、仅偶然撞同一个文件，合并的上下文复用收益小，默认不勾。
const LOW_OVERLAP_RATIO: f64 = 0.34;

/// 单条需求参与探测所需的精简画像（从 spec 抽取）。
#[derive(Debug, Clone)]
pub struct IssueFiles {
    pub id: String,
    pub title: String,
    /// 该需求的全部目标文件（affected_files ∪ files_to_touch，去重）。
    pub files: BTreeSet<String>,
    /// 其中被标记为删除/重命名类改动的文件（用于冲突提示）。
    pub deletes: BTreeSet<String>,
}

/// 文件名是否属于「登记类枢纽文件」：几乎每个需求都会顺带改，合并价值低、易造成虚假聚类。
fn is_builtin_hub(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let base = p.rsplit(['/', '\\']).next().unwrap_or(&p);
    matches!(
        base,
        "lib.rs" | "mod.rs" | "main.rs" | "index.ts" | "index.tsx" | "index.js" | "mod.ts"
    ) || base.ends_with(".md")
        || p.ends_with("services/index.ts")
        || p.contains("/migrations/")
}

/// change_type 是否属于删除/重命名类（与「修改」并存于同一文件时提示潜在冲突）。
fn is_delete_like(change_type: &str) -> bool {
    let c = change_type.to_ascii_lowercase();
    c.contains("delete") || c.contains("remove") || c.contains("rename")
}

/// 纯函数：从需求画像列表算出合并候选组。无 IO、无 Tauri，便于单测。
pub fn compute_candidates(issues: &[IssueFiles]) -> Vec<MergeCandidate> {
    let total = issues.len();
    if total < 2 {
        return Vec::new();
    }

    // 1) 文件命中频率 → 识别频率枢纽。
    let mut freq: HashMap<&str, usize> = HashMap::new();
    for it in issues {
        for f in &it.files {
            *freq.entry(f.as_str()).or_insert(0) += 1;
        }
    }
    let is_hub = |f: &str| -> bool {
        if is_builtin_hub(f) {
            return true;
        }
        let c = *freq.get(f).unwrap_or(&0);
        c >= HUB_MIN_COUNT && c * 2 >= total // ≥50% 且 ≥HUB_MIN_COUNT
    };

    // 2) 每个实质文件 → 命中它的需求下标集合（clique 以「共享同一实质文件」成组）。
    let mut by_file: HashMap<&str, Vec<usize>> = HashMap::new();
    for (idx, it) in issues.iter().enumerate() {
        for f in &it.files {
            if !is_hub(f) {
                by_file.entry(f.as_str()).or_default().push(idx);
            }
        }
    }

    // 3) 成组 + 去重（按成员集合）+ 计算共享文件/blast/冲突/强弱。
    let mut seen_groups: BTreeSet<Vec<usize>> = BTreeSet::new();
    let mut out: Vec<MergeCandidate> = Vec::new();
    for (_file, members) in by_file.iter() {
        if members.len() < 2 || members.len() > MAX_GROUP {
            continue;
        }
        let mut key = members.clone();
        key.sort_unstable();
        key.dedup();
        if key.len() < 2 || !seen_groups.insert(key.clone()) {
            continue;
        }

        // 共享文件 = 组内全体成员文件集合的交集（含触发文件 + 其它全员共有文件）。
        let mut shared: BTreeSet<String> = issues[key[0]].files.clone();
        for &m in &key[1..] {
            shared = shared.intersection(&issues[m].files).cloned().collect();
        }
        // 交集里也滤掉枢纽，避免「仅共享 lib.rs」被当强信号展示。
        let shared_substantive: Vec<String> =
            shared.iter().filter(|f| !is_hub(f)).cloned().collect();
        if shared_substantive.is_empty() {
            continue; // 全员仅共享枢纽文件 → 不成实质候选
        }

        // blast radius = 成员文件并集大小。
        let mut union: BTreeSet<&String> = BTreeSet::new();
        for &m in &key {
            for f in &issues[m].files {
                union.insert(f);
            }
        }
        let total_files = union.len();

        // 冲突提示：某共享文件在一个成员里是删除/重命名、在另一个成员里是修改。
        let mut conflict_files: Vec<String> = Vec::new();
        for f in &shared_substantive {
            let del = key.iter().any(|&m| issues[m].deletes.contains(f));
            let modi = key
                .iter()
                .any(|&m| issues[m].files.contains(f) && !issues[m].deletes.contains(f));
            if del && modi {
                conflict_files.push(f.clone());
            }
        }
        let conflict_hint = if conflict_files.is_empty() {
            None
        } else {
            Some(format!(
                "潜在冲突：{} 在不同需求中既被删除/重命名又被修改",
                conflict_files.join("、")
            ))
        };

        // 重叠占比（设计 §2.4）：共享实质文件相对最小成员文件集的占比。相对小集合，
        // 对「小需求并入大需求」更友好——分母取最小成员，避免大需求拖低占比。
        let min_member_files = key
            .iter()
            .map(|&m| issues[m].files.len())
            .min()
            .unwrap_or(0)
            .max(1);
        let overlap_ratio = shared_substantive.len() as f64 / min_member_files as f64;

        // 强候选需同时满足：blast 不超标、无删改冲突、重叠占比达标。任一不满足 → 弱候选。
        let strength = if total_files <= MAX_BLAST
            && conflict_hint.is_none()
            && overlap_ratio >= LOW_OVERLAP_RATIO
        {
            "strong"
        } else {
            "weak"
        };

        out.push(MergeCandidate {
            issue_ids: key.iter().map(|&m| issues[m].id.clone()).collect(),
            titles: key.iter().map(|&m| issues[m].title.clone()).collect(),
            shared_files: shared_substantive,
            total_files,
            strength: strength.to_string(),
            conflict_hint,
        });
    }

    // 强候选在前；同强度按组大小降序（覆盖更多需求的优先）。
    out.sort_by(|a, b| {
        let sa = (a.strength == "strong") as u8;
        let sb = (b.strength == "strong") as u8;
        sb.cmp(&sa).then(b.issue_ids.len().cmp(&a.issue_ids.len()))
    });
    out
}

/// 从一条需求的分析 spec 抽取文件画像。无 spec / 解析失败 → 文件集为空（不参与成组）。
fn issue_files_from_spec(
    id: String,
    title: String,
    spec: Option<&crate::agents::analysis::IssueAnalysisSpec>,
) -> IssueFiles {
    let mut files: BTreeSet<String> = BTreeSet::new();
    let mut deletes: BTreeSet<String> = BTreeSet::new();
    if let Some(sp) = spec {
        for af in &sp.scope.affected_files {
            let p = af.path.trim();
            if !p.is_empty() {
                files.insert(p.to_string());
                if is_delete_like(&af.change_type) {
                    deletes.insert(p.to_string());
                }
            }
        }
        for f in &sp.claude_code_brief.files_to_touch {
            let p = f.trim();
            if !p.is_empty() {
                files.insert(p.to_string());
            }
        }
    }
    IssueFiles {
        id,
        title,
        files,
        deletes,
    }
}

/// 列出某项目「需求审核」队列里的合并候选组。
#[tauri::command]
pub async fn list_merge_candidates(
    project_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<MergeCandidate>, String> {
    // 待需求审核且有分析结果的需求才可合并（analysis_failed 无 spec、无文件信号，排除）。
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT i.id, i.title, a.analysis_json
         FROM issues i
         LEFT JOIN issue_analyses a ON a.issue_id = i.id
         WHERE i.project_id = ? AND i.status = 'pending_issue_review'
         ORDER BY i.created_at",
    )
    .bind(&project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    let issues: Vec<IssueFiles> = rows
        .into_iter()
        .map(|(id, title, json)| {
            let spec = json
                .as_deref()
                .and_then(crate::agents::analysis::parse_spec);
            issue_files_from_spec(id, title, spec.as_ref())
        })
        .filter(|f| !f.files.is_empty())
        .collect();

    Ok(compute_candidates(&issues))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, files: &[&str]) -> IssueFiles {
        IssueFiles {
            id: id.into(),
            title: id.into(),
            files: files.iter().map(|s| s.to_string()).collect(),
            deletes: BTreeSet::new(),
        }
    }

    #[test]
    fn shared_substantive_file_forms_group() {
        let issues = vec![
            mk("a", &["src/foo.rs", "src/a_only.rs"]),
            mk("b", &["src/foo.rs", "src/b_only.rs"]),
        ];
        let c = compute_candidates(&issues);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].issue_ids.len(), 2);
        assert_eq!(c[0].shared_files, vec!["src/foo.rs".to_string()]);
        assert_eq!(c[0].strength, "strong");
    }

    #[test]
    fn no_shared_file_no_group() {
        let issues = vec![mk("a", &["src/x.rs"]), mk("b", &["src/y.rs"])];
        assert!(compute_candidates(&issues).is_empty());
    }

    #[test]
    fn builtin_hub_only_share_does_not_group() {
        // 仅共享 lib.rs（登记类枢纽）→ 不成组。
        let issues = vec![
            mk("a", &["src/lib.rs", "src/x.rs"]),
            mk("b", &["src/lib.rs", "src/y.rs"]),
        ];
        assert!(compute_candidates(&issues).is_empty());
    }

    #[test]
    fn frequency_hub_is_downweighted() {
        // shared.rs 被 5 条里的 4 条命中（≥50% 且 ≥HUB_MIN_COUNT）→ 枢纽，不据此成组。
        // 但 a、b 另外共享 real.rs（实质）→ 仅 a、b 成一组。
        let issues = vec![
            mk("a", &["shared.rs", "real.rs"]),
            mk("b", &["shared.rs", "real.rs"]),
            mk("c", &["shared.rs"]),
            mk("d", &["shared.rs"]),
            mk("e", &["solo.rs"]),
        ];
        let c = compute_candidates(&issues);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].shared_files, vec!["real.rs".to_string()]);
        let mut ids = c[0].issue_ids.clone();
        ids.sort();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn delete_vs_modify_flags_conflict() {
        let mut a = mk("a", &["src/foo.rs"]);
        a.deletes.insert("src/foo.rs".to_string());
        let b = mk("b", &["src/foo.rs"]);
        let c = compute_candidates(&[a, b]);
        assert_eq!(c.len(), 1);
        assert!(c[0].conflict_hint.is_some());
        assert_eq!(c[0].strength, "weak");
    }

    #[test]
    fn low_overlap_ratio_is_weak() {
        // a、b 各 4 个文件，仅共享 1 个实质文件 → overlap 0.25 < 0.34 → 弱候选（成组但弱化）。
        let issues = vec![
            mk("a", &["src/foo.rs", "src/a1.rs", "src/a2.rs", "src/a3.rs"]),
            mk("b", &["src/foo.rs", "src/b1.rs", "src/b2.rs", "src/b3.rs"]),
        ];
        let c = compute_candidates(&issues);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].shared_files, vec!["src/foo.rs".to_string()]);
        assert_eq!(c[0].strength, "weak");
    }

    #[test]
    fn high_overlap_ratio_is_strong() {
        // 共享 1 个 / 各 2 个 → overlap 0.5 ≥ 0.34 → 强候选（与 shared_substantive_file_forms_group 互补）。
        let issues = vec![
            mk("a", &["src/foo.rs", "src/a.rs"]),
            mk("b", &["src/foo.rs", "src/b.rs"]),
        ];
        let c = compute_candidates(&issues);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].strength, "strong");
    }

    #[test]
    fn oversize_group_skipped() {
        // 6 条共享同一实质文件 → 超 MAX_GROUP，跳过。
        let issues: Vec<IssueFiles> = (0..6)
            .map(|i| mk(&format!("i{i}"), &["src/hot.rs"]))
            .collect();
        // hot.rs 被 6/6 命中 → 也会被频率枢纽规则滤掉，双重保证不成巨簇。
        assert!(compute_candidates(&issues).is_empty());
    }
}
