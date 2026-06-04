PRAGMA foreign_keys=OFF;

DELETE FROM scan_findings;
DELETE FROM admin_decisions;
DELETE FROM preview_environments;
DELETE FROM test_sessions;
DELETE FROM conversation_attachments;
DELETE FROM messages;
DELETE FROM conversation_reads;
DELETE FROM conversation_members;
DELETE FROM conversations;
DELETE FROM worktree_sessions;
DELETE FROM change_requests;
DELETE FROM issue_analyses;
DELETE FROM issues;
DELETE FROM projects;

PRAGMA foreign_keys=ON;
