use once_cell::sync::Lazy;
use regex::Regex;
use sha2::{Digest, Sha256};

static INJECTION_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    [
        "ignore\\s+(all\\s+)?(previous|above|prior)\\s+instructions?",
        "you\\s+are\\s+now\\s+",
        "system\\s+prompt",
        "jailbreak",
        "DAN\\b",
        "forget\\s+(your\\s+)?(rules|constraints|instructions)",
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
