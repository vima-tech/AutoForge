use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct NotifyChannel {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub target: String,
    pub events: String,
    pub enabled: bool,
    #[serde(default)]
    pub secret: String,
    pub created_at: String,
}

/// 对前端暴露的视图：绝不回传明文 secret，仅给出 `has_secret` 指示位。
#[derive(Debug, Clone, Serialize)]
pub struct NotifyChannelView {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub target: String,
    pub events: String,
    pub enabled: bool,
    pub has_secret: bool,
    pub created_at: String,
}

impl From<NotifyChannel> for NotifyChannelView {
    fn from(c: NotifyChannel) -> Self {
        let has_secret = !c.secret.trim().is_empty();
        NotifyChannelView {
            id: c.id,
            name: c.name,
            kind: c.kind,
            target: c.target,
            events: c.events,
            enabled: c.enabled,
            has_secret,
            created_at: c.created_at,
        }
    }
}
