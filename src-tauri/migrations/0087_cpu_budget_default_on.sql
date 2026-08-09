-- 并发调度按核预算 P1：cgroup CPU 硬兜底默认开启。
--
-- 背景：核预算硬兜底（core/cpubudget.rs）代码早已实现且 code agent 进程组已 attach，但
-- 默认 `execution.cpu_budget_pct=0` 关闭 → N 个并行 claude -p 内部 rustc/tsc 突发无硬封顶，
-- 唯一即时生效的只有事后 loadavg 闸，导致并发编码时 CPU 占满。
--
-- 存量 DB 里若曾保存过设置就写入了 '0'（改 Rust 默认常量对这些行无效）。这里把「明确等于 0」
-- 的存量值一次性重置为 90，与新默认对齐。大概率 0 是「从未调过」而非「有意关闭」；用户仍可在
-- 「并发控制」UI 改回 0 显式关闭。仅重置值恰为 '0' 的行，不动其它显式设定值。
UPDATE app_settings
   SET value = '90', updated_at = datetime('now')
 WHERE key = 'execution.cpu_budget_pct' AND value = '0';
