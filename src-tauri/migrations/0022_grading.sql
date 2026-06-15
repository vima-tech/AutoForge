-- §7 Diff risk grading + gate-downgrade trust state machine
CREATE TABLE IF NOT EXISTS cr_grades (
    change_request_id TEXT PRIMARY KEY,
    tier         TEXT NOT NULL DEFAULT 'T2',   -- T0 | T1 | T2 | T3
    score        INTEGER NOT NULL DEFAULT 0,
    rationale    TEXT NOT NULL DEFAULT '',
    change_class TEXT NOT NULL DEFAULT 'general',
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Per change-class trust state (cold→eligible→auto→demoted). Auto-pass disabled
-- out of the box; a class only earns lower friction with measured data.
CREATE TABLE IF NOT EXISTS auto_pass_policy (
    change_class  TEXT PRIMARY KEY,
    trust_state   TEXT NOT NULL DEFAULT 'cold', -- cold | eligible | auto | demoted
    approve_count INTEGER NOT NULL DEFAULT 0,
    reject_count  INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
