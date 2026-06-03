/* ============================================================
   AutoForge — mock data
   ============================================================ */

const AGENTS = [
  { id: 'analyst', name: '需求分析师', en: 'Analysis Agent', role: '需求真实性 · 可行性 · 优先级', color: '#8b7ad8', initial: '析', llm: 'Claude Opus 4', system: '你是 AutoForge 的需求分析 Agent。评估需求真实性、可行性、优先级，输出结构化分析报告，检测重复需求。', forge: 'analysis' },
  { id: 'coder', name: 'Claude Code', en: 'Execution Agent', role: '代码实现 · worktree · 初步测试', color: '#e8772e', initial: 'CC', llm: 'Claude Sonnet 4', system: '你是执行 Agent，在 worktree 内实现已审核的需求，遵守编码规范与权限边界，生成实现报告。', forge: null },
  { id: 'tester', name: '测试工程师', en: 'Test Agent', role: '被动响应 · 主动巡检 · 缺陷报告', color: '#4f9d6b', initial: '测', llm: 'Claude Sonnet 4', system: '你是测试 Agent，验证功能正确性、执行回归与主动巡检，生成缺陷报告并自动入队。', forge: 'test' },
  { id: 'architect', name: '架构顾问', en: 'Architect', role: '技术选型 · 模块设计 · 重构建议', color: '#4f8ed1', initial: '架', llm: 'Claude Opus 4', system: '你是架构顾问，对技术选型、模块边界和重构方案给出建议。', forge: null },
  { id: 'security', name: '安全审计', en: 'Security', role: 'Prompt 注入 · 权限边界 · 脱敏', color: '#db5a40', initial: '安', llm: 'Claude Sonnet 4', system: '你是安全审计 Agent，识别 Prompt 注入、权限越界与数据脱敏风险。', forge: null },
];

const A = Object.fromEntries(AGENTS.map(a => [a.id, a]));

const CONVERSATIONS = [
  {
    id: 'g-vocant', type: 'group', name: 'Vocant · 助手页改版', initial: 'V', color: '#e8772e',
    members: ['analyst', 'coder', 'tester', 'architect'], unread: 2, time: '14:32',
    preview: '需求分析师：已生成需求 #CR-042 的分析报告…',
  },
  {
    id: 'd-analyst', type: 'direct', agent: 'analyst', unread: 0, time: '13:07',
    preview: '这个反馈与 #CR-031 重复度 82%，建议合并处理。',
  },
  {
    id: 'd-coder', type: 'direct', agent: 'coder', unread: 1, time: '11:45',
    preview: 'worktree autoforge/cr-042 已创建，预览构建中…',
  },
  {
    id: 'g-security', type: 'group', name: '安全加固讨论组', initial: '安', color: '#db5a40',
    members: ['security', 'architect', 'coder'], unread: 0, time: '昨天',
    preview: '安全审计：Layer 1 输入消毒需在 M4 前落地。',
  },
  {
    id: 'd-tester', type: 'direct', agent: 'tester', unread: 0, time: '昨天',
    preview: '主动巡检完成，发现 1 个 High、3 个 Medium 级问题。',
  },
  {
    id: 'd-architect', type: 'direct', agent: 'architect', unread: 0, time: '周一',
    preview: '建议预览路由初期用 Nginx reload，瓶颈后再评估 Traefik。',
  },
];

// message content blocks: {t:'md'|'code'|'file'|'image'|'artifact'|'typing'}
const MESSAGES = {
  'g-vocant': [
    { from: 'me', time: '14:18', blocks: [{ t: 'md', md: '各位，用户反馈**助手页缺少标签筛选**，想给对话按场景分类。我先把原始反馈和截图发上来，大家看下可行性。@需求分析师 帮忙评估真实性和优先级。' }] },
    { from: 'me', time: '14:18', blocks: [{ t: 'image', label: 'assistant-page.png', meta: 'PNG · 248 KB · 1440×900', color: '#4f8ed1' }] },
    { from: 'me', time: '14:18', blocks: [{ t: 'file', name: 'feedback-raw.json', meta: 'JSON · 3.2 KB', color: '#e0a32e' }] },
    { from: 'analyst', time: '14:21', blocks: [
      { t: 'md', md: '收到。我分析了这条反馈以及关联的 12 条历史反馈：\n\n- **真实性** 0.91 — 多个用户提及同类诉求，非噪声\n- **可行性** 0.84 — 前端改动为主，无 schema 破坏\n- **重复检测** 与 `#CR-031` 重复度 28%，可独立处理' },
      { t: 'artifact', kind: '需求分析报告', title: 'CR-042 · 助手页标签筛选', rows: [['分类', 'Feature'], ['优先级', 'P2 · 7/10'], ['影响模块', 'Assistant.jsx, feedback.py']], body: '建议在助手页顶部新增标签栏，支持「全部 / 待办 / 已完成 / 自定义」筛选。预估改动 2 个文件，无数据库迁移。' },
    ] },
    { from: 'me', time: '14:24', blocks: [{ t: 'md', md: '分析得不错。@架构顾问 这个标签状态存前端还是后端？@Claude Code 如果走后端要加迁移吗？' }] },
    { from: 'architect', time: '14:27', blocks: [{ t: 'md', md: '建议**前端本地状态 + URL query** 即可，标签是视图层筛选，不需要持久化到后端。这样零迁移、零接口改动，最小化风险。' }] },
    { from: 'coder', time: '14:30', blocks: [
      { t: 'md', md: '同意架构顾问。我按前端方案实现，已读取 `Assistant.jsx` 上下文，核心改动如下：' },
      { t: 'code', lang: 'jsx', code: 'const [tag, setTag] = useState(\n  searchParams.get("tag") || "all"\n);\n\nconst filtered = items.filter(\n  it => tag === "all" || it.status === tag\n);' },
      { t: 'md', md: '我去创建 worktree `autoforge/cr-042` 并跑初步测试，预览就绪后通知大家。' },
    ] },
    { from: 'coder', time: '14:32', blocks: [{ t: 'typing' }] },
  ],
  'd-analyst': [
    { from: 'analyst', time: '12:55', blocks: [{ t: 'md', md: '早。今天入口共进来 **9 条**外部反馈，我已完成首轮分类：\n\n- Feature × 4\n- Bug × 3\n- Improvement × 2' }] },
    { from: 'me', time: '13:02', blocks: [{ t: 'md', md: '那条"导出报表很慢"的反馈呢？' }] },
    { from: 'analyst', time: '13:07', blocks: [
      { t: 'md', md: '这个反馈与 `#CR-031` 重复度 **82%**，建议合并处理。两条都指向 `export_service.py` 的同步导出逻辑。' },
      { t: 'artifact', kind: '重复检测', title: '合并建议 · CR-031', rows: [['原始', '#CR-049'], ['合并至', '#CR-031'], ['相似度', '82%']], body: '建议将 #CR-049 标记为 #CR-031 的重复项，归档处理，避免重复实现。' },
    ] },
  ],
  'd-coder': [
    { from: 'me', time: '11:30', blocks: [{ t: 'md', md: '开始实现 CR-042 吧，注意只改前端，不要动接口。' }] },
    { from: 'coder', time: '11:31', blocks: [{ t: 'md', md: '明白。正在创建隔离 worktree…' }] },
    { from: 'coder', time: '11:45', blocks: [
      { t: 'md', md: 'worktree `autoforge/cr-042` 已创建，预览构建中…' },
      { t: 'artifact', kind: 'WORKTREE', title: 'autoforge/cr-042', rows: [['分支', 'autoforge/cr-042'], ['槽位', '2 / 5'], ['状态', '构建中 · 注入快照库']], body: 'Preview Orchestrator 正在快照预览数据库并启动容器，约 30s 后预览就绪。' },
    ] },
  ],
  'g-security': [
    { from: 'security', time: '昨天 16:02', blocks: [{ t: 'md', md: '重申一下三层防护的落地顺序：**Layer 1 输入消毒必须在 M4 之前**，它不是功能，是前提条件。' }] },
    { from: 'architect', time: '昨天 16:10', blocks: [{ t: 'md', md: 'Layer 1 每条外部输入加一次 LLM 调用，延迟 1–3s。走异步队列用户无感知，成本敏感时可换本地小模型。' }] },
    { from: 'coder', time: '昨天 16:14', blocks: [{ t: 'md', md: 'Git 代理层我会拦截 `git push origin main`、修改 `.git/config` 远端等已知越权操作，配合 Layer 2 行为审计实时终止异常会话。' }] },
  ],
  'd-tester': [
    { from: 'tester', time: '昨天 09:00', blocks: [
      { t: 'md', md: '每日主动巡检完成 ✓\n\n- 全量测试：**312 通过 / 2 失败**\n- 覆盖率：81.3%（阈值 80%）\n- Lint：clean ｜ Typing：clean' },
      { t: 'artifact', kind: '巡检报告', title: '发现 4 个问题', rows: [['High', '1'], ['Medium', '3'], ['Low', '汇入周报']], body: 'High 级问题已自动创建高优先级 Issue 并通知。Medium 进入常规队列，Low 汇入每周质量周报。' },
    ] },
  ],
  'd-architect': [
    { from: 'architect', time: '周一 15:20', blocks: [{ t: 'md', md: '关于预览路由：\n\n> 建议初期用 **Nginx reload**，按 `cr-id` 路由到容器。高频重建场景 reload 可能短暂中断，届时再评估 **Traefik**。' }] },
  ],
};

// ---- Dashboard ----
const DASH = {
  stats: [
    { ic: 'box', color: '#e8772e', val: '4', unit: '', label: '在产项目', delta: '+1 本周', up: true },
    { ic: 'inbox', color: '#8b7ad8', val: '17', unit: '', label: '需求队列', delta: '+6 今日', up: true },
    { ic: 'cpu', color: '#4f8ed1', val: '3', unit: '/5', label: '并发槽位占用', delta: '阶段 1 · 正常', up: true },
    { ic: 'clock', color: '#4f9d6b', val: '1.6', unit: 'h', label: '平均交付时长', delta: '-12% vs 上周', up: true },
  ],
  pipeline: [
    { ic: 'inbox', name: '需求入口', cnt: 9, state: 'done' },
    { ic: 'search', name: '需求分析', cnt: 4, state: 'done' },
    { ic: 'check', name: '审核 1', cnt: 3, state: 'active' },
    { ic: 'code', name: 'Claude Code', cnt: 3, state: 'active' },
    { ic: 'eye', name: '审核 2', cnt: 5, state: 'warn' },
    { ic: 'merge', name: '合并 dev', cnt: 2, state: '' },
  ],
  queue: [
    { pr: 1, title: '助手页缺少标签筛选', id: 'CR-042', cat: 'Feature', sev: 'high', proj: 'Vocant', stage: '实现中' },
    { pr: 2, title: '导出报表接口超时 (>30s)', id: 'CR-031', cat: 'Bug', sev: 'critical', proj: 'Vocant', stage: '待审核 2' },
    { pr: 3, title: '客户列表分页错乱', id: 'CR-048', cat: 'Bug', sev: 'high', proj: 'AnotherApp', stage: '待审核 2' },
    { pr: 4, title: '登录页增加企业微信扫码', id: 'CR-051', cat: 'Feature', sev: 'medium', proj: 'Vocant', stage: '待审核 1' },
    { pr: 5, title: '死代码清理 (utils/legacy)', id: 'CR-053', cat: 'Debt', sev: 'low', proj: 'AutoForge', stage: '待分析' },
  ],
  projects: [
    { name: 'Vocant', color: '#e8772e', desc: 'AI 原生业务管理工具', lang: 'python · fastapi', backlog: 8, status: 'active' },
    { name: 'AnotherApp', color: '#4f8ed1', desc: '内部协作平台', lang: 'node · next.js', backlog: 5, status: 'active' },
    { name: 'AutoForge', color: '#8b7ad8', desc: '工厂自进化（固定框架）', lang: 'python · fastapi', backlog: 3, status: 'active' },
    { name: 'Landing', color: '#4f9d6b', desc: '官网静态站', lang: 'static · nginx', backlog: 1, status: 'paused' },
  ],
};

// ---- Audit ----
const AUDIT_PROJECTS = [
  { name: 'Vocant', color: '#e8772e', lang: 'python · fastapi', preview: 'localhost:9000/vocant/dev', live: true },
  { name: 'AnotherApp', color: '#4f8ed1', lang: 'node · next.js', preview: 'localhost:9000/another/dev', live: true },
  { name: 'AutoForge', color: '#8b7ad8', lang: 'python · fastapi', preview: 'localhost:9000/autoforge/dev', live: false },
];

const REQUIREMENTS = [
  { id: 'CR-042', title: '助手页缺少标签筛选', cat: 'Feature', sev: 'high', iter: 1, files: 2, author: '需求分析师', time: '8 分钟前' },
  { id: 'CR-031', title: '导出报表接口超时 (>30s)', cat: 'Bug', sev: 'critical', iter: 3, files: 4, author: 'Claude Code', time: '26 分钟前' },
  { id: 'CR-051', title: '登录页增加企业微信扫码', cat: 'Feature', sev: 'medium', iter: 2, files: 5, author: 'Claude Code', time: '1 小时前' },
  { id: 'CR-049', title: '商品库存为负时的兜底校验', cat: 'Bug', sev: 'high', iter: 1, files: 3, author: '测试工程师', time: '2 小时前' },
];

const REPORT = {
  summary: '为助手页新增标签筛选栏，支持「全部 / 待办 / 已完成」三种视图过滤。改动集中在前端视图层，使用本地状态 + URL query 持久化筛选条件，不涉及后端接口与数据库迁移。',
  files: [
    { name: 'app/web/Assistant.jsx', add: 34, del: 6 },
    { name: 'app/web/styles/assistant.css', add: 18, del: 0 },
  ],
  tests: { added: 3, passed: true, cov: '81.3%' },
  risk: '无破坏性变更。标签状态仅作用于前端视图，刷新后由 URL query 恢复，不影响其它模块。',
};

const DIFF_HUNKS = [
  { file: 'app/web/Assistant.jsx', hunk: '@@ -12,6 +12,18 @@ export function Assistant() {', lines: [
    { n1: 12, n2: 12, t: 'ctx', code: '  const { items } = useAssistant();' },
    { n1: 13, n2: 13, t: 'ctx', code: '  const [search, setSearch] = useState("");' },
    { n1: '', n2: 14, t: 'add', code: '  const [params, setParams] = useSearchParams();' },
    { n1: '', n2: 15, t: 'add', code: '  const tag = params.get("tag") || "all";' },
    { n1: '', n2: 16, t: 'add', code: '  const setTag = (t) =>' },
    { n1: '', n2: 17, t: 'add', code: '    setParams(t === "all" ? {} : { tag: t });' },
    { n1: 14, n2: 18, t: 'ctx', code: '' },
    { n1: 15, n2: 19, t: 'del', code: '  const filtered = items.filter(matchSearch);' },
    { n1: '', n2: 20, t: 'add', code: '  const filtered = items' },
    { n1: '', n2: 21, t: 'add', code: '    .filter(it => tag === "all" || it.status === tag)' },
    { n1: '', n2: 22, t: 'add', code: '    .filter(matchSearch);' },
    { n1: 16, n2: 23, t: 'ctx', code: '' },
    { n1: 17, n2: 24, t: 'ctx', code: '  return (' },
  ]},
  { file: 'app/web/Assistant.jsx', hunk: '@@ -28,0 +35,8 @@ return (', lines: [
    { n1: 28, n2: 35, t: 'ctx', code: '    <div className="assistant">' },
    { n1: '', n2: 36, t: 'add', code: '      <nav className="tag-bar">' },
    { n1: '', n2: 37, t: 'add', code: '        {TAGS.map(t => (' },
    { n1: '', n2: 38, t: 'add', code: '          <button key={t.id}' },
    { n1: '', n2: 39, t: 'add', code: '            className={tag === t.id ? "on" : ""}' },
    { n1: '', n2: 40, t: 'add', code: '            onClick={() => setTag(t.id)}>{t.label}</button>' },
    { n1: '', n2: 41, t: 'add', code: '        ))}' },
    { n1: '', n2: 42, t: 'add', code: '      </nav>' },
    { n1: 29, n2: 43, t: 'ctx', code: '      <SearchBox value={search} onChange={setSearch} />' },
  ]},
];

// ---- Settings ----
const LLM_CONFIGS = [
  { id: 'l1', name: 'Claude Opus 4', provider: 'Anthropic', color: '#e8772e', model: 'claude-opus-4-20250514', endpoint: 'https://api.anthropic.com', key: 'sk-ant-•••••••••••••4a2f', ctx: '200K', temp: '0.3', active: true },
  { id: 'l2', name: 'Claude Sonnet 4', provider: 'Anthropic', color: '#8b7ad8', model: 'claude-sonnet-4-20250514', endpoint: 'https://api.anthropic.com', key: 'sk-ant-•••••••••••••9c1b', ctx: '200K', temp: '0.2', active: true },
  { id: 'l3', name: '本地消毒模型', provider: 'Ollama', color: '#4f9d6b', model: 'qwen2.5:7b-instruct', endpoint: 'http://localhost:11434', key: '—', ctx: '32K', temp: '0.0', active: true },
];

window.AF = { AGENTS, A, CONVERSATIONS, MESSAGES, DASH, AUDIT_PROJECTS, REQUIREMENTS, REPORT, DIFF_HUNKS, LLM_CONFIGS };
