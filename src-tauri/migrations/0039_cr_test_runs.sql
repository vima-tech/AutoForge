-- CR 级测试遥测（design 阶段 D）：review_2 合并前自动跑的**项目级**测试结果落一条记录，
-- 作为工厂吞吐质量遥测。刻意只记 CR 整体 pass/fail，不做逐用例映射/通过率/用例库（非 PM 工具）。

CREATE TABLE IF NOT EXISTS cr_test_runs (
    id      TEXT PRIMARY KEY,
    cr_id   TEXT NOT NULL,
    result  TEXT NOT NULL,                 -- 'pass' | 'fail'
    summary TEXT NOT NULL DEFAULT '',
    run_by  TEXT NOT NULL DEFAULT 'auto',
    run_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_cr_test_runs_cr ON cr_test_runs(cr_id);
