use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConversationTask {
    pub id: String,
    pub conversation_id: String,
    pub trigger_message_id: String,
    pub planner_agent_id: Option<String>,
    pub instruction: String,
    pub plan_json: String,
    pub status: String,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[allow(dead_code)]
pub struct ConversationTaskStep {
    pub id: String,
    pub task_id: String,
    pub step_index: i64,
    pub step_type: String,
    pub agent_ids_json: String,
    pub instruction: String,
    pub status: String,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[allow(dead_code)]
pub struct ConversationTaskRun {
    pub id: String,
    pub task_id: String,
    pub step_id: String,
    pub agent_id: String,
    pub status: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub message_id: Option<String>,
    pub output_text: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartConversationTask {
    pub conversation_id: String,
    pub trigger_message_id: String,
    pub instruction: String,
    pub mentioned_agent_ids: Vec<String>,
    pub window_size: Option<i64>,
}
