-- 修正存量「新需求录入」通知的跳转目标页。
-- 历史上 issue_created 的 link_page 被错填为 'delivery'（交付流水线），
-- 但新需求（待需求审核的 issue）实际展示与审核位置是「功能审计」页，
-- 故将存量记录统一改为 'audit'，与 models/notification.rs 的最新映射保持一致。
UPDATE notifications SET link_page = 'audit' WHERE kind = 'issue_created' AND link_page = 'delivery';
