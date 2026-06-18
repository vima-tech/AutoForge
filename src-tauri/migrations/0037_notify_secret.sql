-- 通知通道：新增加密 secret 列，承载签名密钥 / Bearer token。
-- target 仍存目标 URL（明文）；secret 经 core/secrets.rs 信封加密为 enc:v1: 密文落库。
-- kind 取值扩展：slack | wecom | feishu | dingtalk | ntfy | clawbot | email | webhook
ALTER TABLE notify_channels ADD COLUMN secret TEXT NOT NULL DEFAULT '';
