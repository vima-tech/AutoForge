-- 审核状态值改名：pending_review_1 → pending_issue_review（需求审核）、
-- pending_review_2 → pending_code_review（代码审核）。两个表都更新（不匹配则无副作用）。
UPDATE issues          SET status='pending_issue_review' WHERE status='pending_review_1';
UPDATE issues          SET status='pending_code_review'  WHERE status='pending_review_2';
UPDATE change_requests SET status='pending_issue_review' WHERE status='pending_review_1';
UPDATE change_requests SET status='pending_code_review'  WHERE status='pending_review_2';
