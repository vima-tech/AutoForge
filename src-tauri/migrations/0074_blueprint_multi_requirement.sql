-- 孵化台（原「蓝图」）从「每项目一份蓝图」升级为「每项目多条大需求」。
-- 每条大需求 = 一个草稿（PRD/规格/任务/对话），且可一键「编码开发」：直接落为一条 issue
-- + CR + 编码执行（跳过前置需求审核，代码审核 review_2 仍为合并唯一闸门）。
-- 新增三列把草稿与它派生的执行工单关联起来，供列表回链状态（编码中/待审核/已实现）。

ALTER TABLE blueprint_drafts ADD COLUMN title    TEXT NOT NULL DEFAULT '';  -- 大需求标题（取自首行/PRD，列表展示用）
ALTER TABLE blueprint_drafts ADD COLUMN issue_id TEXT NOT NULL DEFAULT '';  -- 编码开发后派生的 issue
ALTER TABLE blueprint_drafts ADD COLUMN cr_id    TEXT NOT NULL DEFAULT '';  -- 派生的变更请求（用于状态回链）
