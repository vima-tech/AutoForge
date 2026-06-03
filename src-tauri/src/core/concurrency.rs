use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcurrencyStatus {
    pub active_slots: usize,
    pub max_slots: usize,
    pub pending_review: usize,
    pub pause_threshold: usize,
    pub stage: String,
}

pub struct ConcurrencyManager {
    semaphore: Arc<Semaphore>,
    max_slots: Arc<Mutex<usize>>,
    pending_review: Arc<Mutex<usize>>,
    pause_threshold: Arc<Mutex<usize>>,
    queue_strategy: Arc<Mutex<String>>,
    reserved_permits: Arc<Mutex<Vec<OwnedSemaphorePermit>>>,
}

impl ConcurrencyManager {
    pub fn new(max_slots: usize, pause_threshold: usize) -> Arc<Self> {
        Arc::new(Self {
            semaphore: Arc::new(Semaphore::new(max_slots)),
            max_slots: Arc::new(Mutex::new(max_slots)),
            pending_review: Arc::new(Mutex::new(0)),
            pause_threshold: Arc::new(Mutex::new(pause_threshold)),
            queue_strategy: Arc::new(Mutex::new("priority".to_string())),
            reserved_permits: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub async fn try_acquire(&self) -> Option<OwnedSemaphorePermit> {
        let pending = {
            let guard = self.pending_review.lock().unwrap();
            *guard
        };
        let pause_threshold = {
            let guard = self.pause_threshold.lock().unwrap();
            *guard
        };
        if pending >= pause_threshold {
            return None;
        }

        let status = self.status();
        if status.active_slots >= status.max_slots {
            return None;
        }
        self.semaphore.clone().try_acquire_owned().ok()
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
        let pending = {
            let guard = self.pending_review.lock().unwrap();
            *guard
        };
        let max_slots = {
            let guard = self.max_slots.lock().unwrap();
            *guard
        };
        let pause_threshold = {
            let guard = self.pause_threshold.lock().unwrap();
            *guard
        };
        let available = self.semaphore.available_permits();
        let active_slots = max_slots.saturating_sub(available);

        let stage = if pending >= pause_threshold {
            "paused".to_string()
        } else if pending >= pause_threshold / 2 {
            "throttled".to_string()
        } else {
            "normal".to_string()
        };

        ConcurrencyStatus {
            active_slots,
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
            let new_max = new_max.max(1);
            let mut current = self.max_slots.lock().unwrap();
            if new_max < *current {
                let shrink_by = *current - new_max;
                let mut reserved = self.reserved_permits.lock().unwrap();
                for _ in 0..shrink_by {
                    if let Ok(permit) = self.semaphore.clone().try_acquire_owned() {
                        reserved.push(permit);
                    }
                }
            } else if new_max > *current {
                let mut grow_by = new_max - *current;
                let mut reserved = self.reserved_permits.lock().unwrap();
                while grow_by > 0 {
                    if reserved.pop().is_some() {
                        grow_by -= 1;
                    } else {
                        break;
                    }
                }
                if grow_by > 0 {
                    self.semaphore.add_permits(grow_by);
                }
            }
            *current = new_max;
        }

        if let Some(new_threshold) = pause_threshold {
            let mut guard = self.pause_threshold.lock().unwrap();
            *guard = new_threshold.max(1);
        }

        if let Some(strategy) = queue_strategy {
            let normalized = match strategy.as_str() {
                "fifo" | "priority" | "oldest" => strategy,
                _ => "priority".to_string(),
            };
            let mut guard = self.queue_strategy.lock().unwrap();
            *guard = normalized;
        }

        self.status()
    }

    pub fn queue_strategy(&self) -> String {
        let guard = self.queue_strategy.lock().unwrap();
        guard.clone()
    }
}
