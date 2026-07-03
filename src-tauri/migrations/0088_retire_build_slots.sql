-- 并发调度按核预算 P2：退役旧「构建池」设置键。
--
-- `execution.build_slots`（1 CR = 1 permit 的 CR 计数）已被 `execution.cpu_permits`（按核加权
-- 令牌上限）取代，语义不同——旧值（默认 2）在核加权语义下等于只给 2 核、限流过紧，故**不迁移
-- 旧值**，直接删除废弃键，让 `load_cpu_permits` 回落到「无值 = nproc」的新默认，自动贴合机器核数。
DELETE FROM app_settings WHERE key = 'execution.build_slots';
