use once_cell::sync::Lazy;
use regex::Regex;
use sha2::{Digest, Sha256};

static INJECTION_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    [
        "ignore\\s+(all\\s+)?(previous|above|prior)\\s+instructions?",
        "disregard\\s+(all\\s+)?(previous|above|prior)\\s+instructions?",
        "you\\s+are\\s+now\\s+",
        "system\\s+prompt",
        "(reveal|print|show|repeat|leak)\\s+(your\\s+|the\\s+)?(system\\s+)?(prompt|instructions)",
        "jailbreak",
        "DAN\\b",
        "developer\\s+mode",
        "forget\\s+(your\\s+|all\\s+)?(rules|constraints|instructions)",
        "(new|updated)\\s+instructions\\s*:",
        "(act|behave|respond)\\s+as\\s+(if|though|an?)\\b",
        "pretend\\s+(to\\s+be|you\\s+are)\\b",
        "<\\|?(im_start|im_end|system|endoftext)\\|?>",
        "\\[/?(INST|SYS)\\]",
    ]
    .iter()
    .map(|p| Regex::new(&format!("(?i){}", p)).unwrap())
    .collect()
});

pub fn has_obvious_injection(text: &str) -> bool {
    INJECTION_PATTERNS.iter().any(|p| p.is_match(text))
}

pub fn fingerprint(title: &str, description: &str) -> String {
    let text = format!(
        "{}|{}",
        title.trim().to_lowercase(),
        description.trim().to_lowercase()
    );
    hex::encode(&Sha256::digest(text.as_bytes())[..16])
}

pub fn safe_truncate(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// 把标题归一化为字符 bigram 集合（shingles），用于近似去重的相似度计算。
/// 先小写化、只保留字母数字与 CJK（丢弃所有标点/空白/符号），再切成相邻 2-gram。
/// bigram 对中英文统一适用（中文无分词器时是廉价而有效的近似），措辞小变动仍高度重合。
pub fn title_shingles(title: &str) -> std::collections::BTreeSet<String> {
    let norm: Vec<char> = title
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    let mut set = std::collections::BTreeSet::new();
    if norm.len() < 2 {
        if !norm.is_empty() {
            set.insert(norm.into_iter().collect());
        }
        return set;
    }
    for w in norm.windows(2) {
        set.insert(w.iter().collect::<String>());
    }
    set
}

/// 两个 shingle 集合的 Jaccard 相似度（交/并），范围 0.0~1.0；任一为空返回 0。
pub fn jaccard(
    a: &std::collections::BTreeSet<String>,
    b: &std::collections::BTreeSet<String>,
) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count();
    let union = a.len() + b.len() - inter;
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shingles_ignore_punctuation_and_case() {
        // 标点/空白/大小写不影响 shingle 集合。
        assert_eq!(title_shingles("ABC"), title_shingles("a b-c!"));
    }

    #[test]
    fn jaccard_identical_is_one_disjoint_is_zero() {
        let a = title_shingles("修复并发竞态问题");
        assert!((jaccard(&a, &a) - 1.0).abs() < 1e-9);
        let b = title_shingles("新增成本统计面板");
        assert!(jaccard(&a, &b) < 0.2);
    }

    #[test]
    fn jaccard_reworded_near_dup_crosses_threshold() {
        // 同一问题换措辞：「修复 X 快照不一致问题」vs「X 快照不一致」应高相似（≥0.72）。
        let a = title_shingles("修复 wait_for_execution_slot 快照不一致问题");
        let b = title_shingles("wait_for_execution_slot 快照不一致");
        assert!(jaccard(&a, &b) >= 0.72, "got {}", jaccard(&a, &b));
    }

    #[test]
    fn jaccard_distinct_requirements_below_threshold() {
        // 同文件不同缺陷不应误判为重复。
        let a = title_shingles("修复 settings.rs 角色分配 TOCTOU 竞态");
        let b = title_shingles("修复 settings.rs 配置更新内存与DB不一致");
        assert!(jaccard(&a, &b) < 0.72, "got {}", jaccard(&a, &b));
    }

    #[test]
    fn empty_titles_are_not_similar() {
        assert_eq!(jaccard(&title_shingles(""), &title_shingles("x")), 0.0);
    }
}
