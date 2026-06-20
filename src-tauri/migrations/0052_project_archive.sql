-- 软删除：项目删除改为归档，保留 DB 数据，配合 .autoforge/project.json 身份锚重新挂回。
-- archived_at 为 NULL = 在用；非空 = 已归档（回收站）。
ALTER TABLE projects ADD COLUMN archived_at TEXT;
