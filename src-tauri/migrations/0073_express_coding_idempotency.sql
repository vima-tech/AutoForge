-- 会议室「立即编码」幂等保护：前端传入 client_request_id（UUID），
-- 后端查重时用唯一索引防止同一请求重复创建 Issue/CR。
-- 仅对非 NULL 值施加唯一约束；普通录入不传 client_request_id，行为不变。
ALTER TABLE issues ADD COLUMN client_request_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_issues_client_request_id
    ON issues (client_request_id)
    WHERE client_request_id IS NOT NULL;
