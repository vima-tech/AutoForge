//! 进程内滑动窗口限流（零外部依赖，纯 Rust）。
//!
//! 用于 `/webhook/issues`：widget token 明文嵌在被嵌入页面 HTML 里，本质不是秘密，
//! 拿着合法 token 也能用 curl 绕过浏览器恶意灌入。token 只解决认证/吊销，**拦不住量**——
//! 限流是真正与 token 是否泄露无关的那道防线。
//!
//! 按多维 key 限流（per-IP / per-project / global），同一实例不同 key 各自独立计数。
//! 内存有界：每次 check 顺手淘汰过期时间戳，并周期性清扫空桶。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 默认窗口与各维度上限（每窗口允许的请求数）。
pub const WINDOW: Duration = Duration::from_secs(60);
/// 单 IP：60s 内最多 20 条（约每 3s 一条，足够真人反馈，挡住脚本刷量）。
pub const IP_MAX: usize = 20;
/// 单项目：60s 内最多 60 条（防分布式 IP 对同一项目灌入）。
pub const PROJECT_MAX: usize = 60;
/// 全局：60s 内最多 300 条（保护整机不被打满 AI/DB）。
pub const GLOBAL_MAX: usize = 300;

/// 超过此 key 数量时触发一次全量清扫，回收已无活跃时间戳的桶。
const SWEEP_THRESHOLD: usize = 4096;

pub struct RateLimiter {
    window: Duration,
    buckets: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// 返回 `true` 表示放行并已计入；`false` 表示该 key 在当前窗口已超 `max`。
    pub fn check(&self, key: &str, max: usize) -> bool {
        let now = Instant::now();
        let mut map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());

        if map.len() > SWEEP_THRESHOLD {
            let window = self.window;
            map.retain(|_, dq| {
                while let Some(&front) = dq.front() {
                    if now.duration_since(front) > window {
                        dq.pop_front();
                    } else {
                        break;
                    }
                }
                !dq.is_empty()
            });
        }

        let dq = map.entry(key.to_string()).or_default();
        while let Some(&front) = dq.front() {
            if now.duration_since(front) > self.window {
                dq.pop_front();
            } else {
                break;
            }
        }
        if dq.len() >= max {
            return false;
        }
        dq.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_max_in_window() {
        let rl = RateLimiter::new(Duration::from_secs(60));
        for _ in 0..3 {
            assert!(rl.check("ip:1.2.3.4", 3));
        }
        // 第 4 次超限
        assert!(!rl.check("ip:1.2.3.4", 3));
        // 不同 key 互不影响
        assert!(rl.check("ip:5.6.7.8", 3));
    }

    #[test]
    fn expired_timestamps_free_capacity() {
        let rl = RateLimiter::new(Duration::from_millis(20));
        assert!(rl.check("k", 1));
        assert!(!rl.check("k", 1));
        std::thread::sleep(Duration::from_millis(30));
        assert!(rl.check("k", 1));
    }
}
