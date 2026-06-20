use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcurrencyStatus {
    pub active_slots: usize,
    pub max_slots: usize,
    pub pending_review: usize,
    pub pause_threshold: usize,
    pub stage: String,
}

/// In-memory accounting for the pipeline. The authoritative admission control
/// (which CR may start executing) is enforced atomically in SQL by the task
/// runner; these counters exist so the dashboard can report live slot/queue
/// usage without re-querying the database on every status call.
pub struct ConcurrencyManager {
    max_slots: Mutex<usize>,
    /// CRs currently in the `executing` phase.
    active: Mutex<usize>,
    /// CRs sitting in `pending_code_review` awaiting human review.
    pending_review: Mutex<usize>,
    pause_threshold: Mutex<usize>,
    queue_strategy: Mutex<String>,
}

impl ConcurrencyManager {
    pub fn new(max_slots: usize, pause_threshold: usize) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            max_slots: Mutex::new(max_slots.max(1)),
            active: Mutex::new(0),
            pending_review: Mutex::new(0),
            pause_threshold: Mutex::new(pause_threshold.max(1)),
            queue_strategy: Mutex::new("priority".to_string()),
        })
    }

    /// A CR has begun executing (a slot is now occupied).
    pub fn slot_acquired(&self) {
        let mut g = self.active.lock().unwrap();
        *g += 1;
    }

    /// A CR has finished executing (the slot is freed).
    pub fn slot_released(&self) {
        let mut g = self.active.lock().unwrap();
        if *g > 0 {
            *g -= 1;
        }
    }

    pub fn transition_to_pending_review(&self) {
        let mut guard = self.pending_review.lock().unwrap();
        *guard += 1;
    }

    pub fn release_pending_review(&self) {
        let mut guard = self.pending_review.lock().unwrap();
        if *guard > 0 {
            *guard -= 1;
        }
    }

    pub fn status(&self) -> ConcurrencyStatus {
        let active = *self.active.lock().unwrap();
        let pending = *self.pending_review.lock().unwrap();
        let max_slots = *self.max_slots.lock().unwrap();
        let pause_threshold = *self.pause_threshold.lock().unwrap();

        let stage = if pending >= pause_threshold {
            "paused".to_string()
        } else if pending >= pause_threshold / 2 {
            "throttled".to_string()
        } else {
            "normal".to_string()
        };

        ConcurrencyStatus {
            active_slots: active,
            max_slots,
            pending_review: pending,
            pause_threshold,
            stage,
        }
    }

    pub fn update_config(
        &self,
        max_slots: Option<usize>,
        pause_threshold: Option<usize>,
        queue_strategy: Option<String>,
    ) -> ConcurrencyStatus {
        if let Some(new_max) = max_slots {
            *self.max_slots.lock().unwrap() = new_max.max(1);
        }
        if let Some(new_threshold) = pause_threshold {
            *self.pause_threshold.lock().unwrap() = new_threshold.max(1);
        }
        if let Some(strategy) = queue_strategy {
            let normalized = match strategy.as_str() {
                "fifo" | "priority" | "oldest" => strategy,
                _ => "priority".to_string(),
            };
            *self.queue_strategy.lock().unwrap() = normalized;
        }
        self.status()
    }

    pub fn queue_strategy(&self) -> String {
        self.queue_strategy.lock().unwrap().clone()
    }
}
