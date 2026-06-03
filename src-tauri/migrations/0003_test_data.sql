INSERT OR IGNORE INTO projects (id, name, slug, description, repo_path, branch_dev, branch_main, status) VALUES
  ('proj-vocant',    'Vocant',     'vocant',     'AI 原生业务管理工具',   '/tmp/vocant',    'dev','main','active'),
  ('proj-another',   'AnotherApp', 'another-app','内部协作平台',          '/tmp/another',   'dev','main','active'),
  ('proj-autoforge', 'AutoForge',  'autoforge',  '工厂自进化（固定框架）','/tmp/autoforge', 'dev','main','active'),
  ('proj-landing',   'Landing',    'landing',    '官网静态站',             '/tmp/landing',   'dev','main','paused');

INSERT OR IGNORE INTO issues (id, project_id, source_type, title, description, category, severity, priority, status, fingerprint) VALUES
  ('issue-001','proj-vocant',   'manual','助手页缺少标签筛选',       '用户反馈对话无法按场景分类','Feature','high',    7,'pending_review_2','fp001'),
  ('issue-002','proj-vocant',   'manual','导出报表接口超时 (>30s)',   '大数据量导出时请求超时',    'Bug',    'critical',9,'pending_review_2','fp002'),
  ('issue-003','proj-another',  'manual','客户列表分页错乱',          '第2页以后数据重复显示',     'Bug',    'high',    8,'pending_review_2','fp003'),
  ('issue-004','proj-vocant',   'manual','登录页增加企业微信扫码',    '企业用户需要微信扫码登录',  'Feature','medium',  5,'pending_review_1','fp004'),
  ('issue-005','proj-autoforge','manual','死代码清理 (utils/legacy)', '清理无用历史代码',          'Debt',   'low',     2,'pending_analysis', 'fp005'),
  ('issue-006','proj-vocant',   'manual','商品库存为负时的兜底校验',  '库存扣减没有做最小值校验',  'Bug',    'high',    8,'executing',        'fp006');

INSERT OR IGNORE INTO issue_analyses (id, issue_id, authenticity_score, feasibility_score, priority_suggestion, category_suggestion, severity_suggestion, affected_modules, analysis_summary) VALUES
  ('anal-001','issue-001',0.91,0.84,7,'Feature','high',   '["Assistant.jsx","feedback.py"]',         '前端标签栏改动为主，无 schema 变化，风险低'),
  ('anal-002','issue-002',0.95,0.70,9,'Bug',    'critical','["export_service.py","celery_tasks.py"]','同步导出逻辑阻塞，需改为异步任务队列'),
  ('anal-003','issue-003',0.88,0.90,8,'Bug',    'high',    '["CustomerList.tsx","api/customers.py"]','分页 offset 计算错误，简单修复'),
  ('anal-004','issue-004',0.75,0.65,5,'Feature','medium',  '["LoginPage.tsx","auth/wechat.py"]',     '需接入企业微信 OAuth，依赖外部 API');

INSERT OR IGNORE INTO change_requests (id, project_id, issue_id, status, admin_id, admin_suggestions_1, target_branch) VALUES
  ('cr-001','proj-vocant', 'issue-001','pending_review_2','admin','只改前端，不要动后端接口','dev'),
  ('cr-002','proj-vocant', 'issue-002','pending_review_2','admin','改为 Celery 异步任务',   'dev'),
  ('cr-003','proj-another','issue-003','pending_review_2','admin',NULL,                      'dev'),
  ('cr-004','proj-vocant', 'issue-006','executing',       'admin',NULL,                      'dev');

INSERT OR IGNORE INTO worktree_sessions (id, change_request_id, worktree_path, branch_name, status, iteration_count, report_content, started_at, completed_at) VALUES
  ('ws-001','cr-001','/tmp/wt/cr-001','autoforge/cr-001','completed',1,
   '## 改动摘要
为助手页新增标签筛选栏，支持全部/待办/已完成三种视图过滤。改动集中在前端视图层。

## 修改文件列表
- app/web/Assistant.jsx: 新增标签栏组件 (+34 -6)
- app/web/styles/assistant.css: 标签栏样式 (+18)

## 测试情况
- 新增测试：3 个
- 通过状态：全部通过 ✓
- 覆盖率：81.3%（阈值 80%）

## 潜在风险
无破坏性变更。标签状态仅作用于前端视图。',
   datetime('now','-2 hours'),datetime('now','-1 hour')),
  ('ws-002','cr-002','/tmp/wt/cr-002','autoforge/cr-002','completed',3,
   '## 改动摘要
将导出接口改为 Celery 异步任务，前端轮询任务状态，完成后提供下载链接。

## 修改文件列表
- export_service.py: 重构为异步任务 (+67 -23)
- api/exports.py: 新增 task_status 端点 (+18)
- frontend/Export.tsx: 改为轮询模式 (+45 -12)

## 测试情况
- 新增测试：6 个
- 通过状态：全部通过 ✓
- 覆盖率：79.1%（低于阈值！）

## 潜在风险
覆盖率略低于阈值，建议补充边界测试。',
   datetime('now','-3 hours'),datetime('now','-30 minutes')),
  ('ws-003','cr-003','/tmp/wt/cr-003','autoforge/cr-003','completed',1,
   '## 改动摘要
修复分页 offset 计算错误，改用 cursor-based 分页防止重复。

## 修改文件列表
- api/customers.py: 修改分页逻辑 (+8 -3)

## 测试情况
- 新增测试：2 个
- 通过状态：全部通过 ✓
- 覆盖率：83.7%

## 潜在风险
无。',
   datetime('now','-1 hour'),datetime('now','-20 minutes'));

INSERT OR IGNORE INTO conversations (id, type, name, color, initial) VALUES
  ('conv-g1','group','Vocant · 助手页改版','#e8772e','V'),
  ('conv-g2','group','安全加固讨论组',     '#db5a40','安'),
  ('conv-d-analyst',  'direct',NULL,'#8b7ad8',NULL),
  ('conv-d-coder',    'direct',NULL,'#e8772e',NULL),
  ('conv-d-tester',   'direct',NULL,'#4f9d6b',NULL),
  ('conv-d-architect','direct',NULL,'#4f8ed1',NULL);

INSERT OR IGNORE INTO conversation_members (conversation_id, agent_id) VALUES
  ('conv-g1','agent-analyst'),('conv-g1','agent-coder'),('conv-g1','agent-tester'),('conv-g1','agent-architect'),
  ('conv-g2','agent-security'),('conv-g2','agent-architect'),('conv-g2','agent-coder'),
  ('conv-d-analyst','agent-analyst'),('conv-d-coder','agent-coder'),
  ('conv-d-tester','agent-tester'),('conv-d-architect','agent-architect');

INSERT OR IGNORE INTO messages (id, conversation_id, from_agent, content_json, created_at) VALUES
  ('msg-001','conv-g1',NULL,
   '[{"t":"md","md":"各位，用户反馈**助手页缺少标签筛选**。@需求分析师 帮忙评估真实性和优先级。"}]',
   datetime('now','-2 hours')),
  ('msg-002','conv-g1','agent-analyst',
   '[{"t":"md","md":"收到。真实性 0.91，可行性 0.84，优先级 P2。"},{"t":"artifact","kind":"需求分析报告","title":"CR-001 · 助手页标签筛选","rows":[["分类","Feature"],["优先级","P2 · 7/10"]],"body":"建议新增标签栏，无数据库迁移。"}]',
   datetime('now','-100 minutes')),
  ('msg-003','conv-g1','agent-coder',
   '[{"t":"md","md":"我按前端方案实现完成，worktree 已创建，预览构建中…"},{"t":"code","lang":"jsx","code":"const [tag, setTag] = useState(\"all\");"}]',
   datetime('now','-90 minutes')),
  ('msg-004','conv-d-analyst','agent-analyst',
   '[{"t":"md","md":"今天入口共进来 **9 条**外部反馈，已完成首轮分类：Feature×4, Bug×3, Improvement×2"}]',
   datetime('now','-3 hours')),
  ('msg-005','conv-d-analyst',NULL,
   '[{"t":"md","md":"那条「导出报表很慢」的反馈呢？"}]',
   datetime('now','-170 minutes')),
  ('msg-006','conv-d-analyst','agent-analyst',
   '[{"t":"md","md":"这个反馈与 `issue-002` 重复度 **82%**，建议合并处理。"}]',
   datetime('now','-165 minutes')),
  ('msg-007','conv-d-coder','agent-coder',
   '[{"t":"md","md":"worktree `autoforge/cr-001` 已创建，预览构建中…"},{"t":"artifact","kind":"WORKTREE","title":"autoforge/cr-001","rows":[["分支","autoforge/cr-001"],["槽位","1 / 5"],["状态","构建中"]],"body":"约 30s 后预览就绪。"}]',
   datetime('now','-95 minutes'));
