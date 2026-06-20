-- 默认项目：让其他页面优先显示/置顶该项目。
-- 约束由命令层保证全表至多一行 is_default=1。
ALTER TABLE projects ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0;
