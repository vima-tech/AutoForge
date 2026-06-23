/**
 * AutoForge IPC service layer.
 * All functions call Tauri invoke() commands; if running in browser (no Tauri),
 * they throw an error that pages should handle gracefully.
 */
import { invoke } from '@tauri-apps/api/core';

// Tracks how many IPC calls are currently in-flight per command.
const _inFlight: Record<string, number> = {};

// 高频/流式命令：每帧音频都会触发，逐条打日志会刷爆控制台且 args 含大体积 base64。
// 这类命令跳过 debug 日志（错误仍会上报）。
const _quietCmds = new Set<string>(['asr_realtime_feed']);

function ipc<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const t0 = performance.now();
  _inFlight[cmd] = (_inFlight[cmd] ?? 0) + 1;
  const quiet = _quietCmds.has(cmd);
  if (!quiet) console.debug(`[IPC] → ${cmd} (in-flight: ${_inFlight[cmd]})`, args ?? '');
  return invoke<T>(cmd, args)
    .then(result => {
      const ms = (performance.now() - t0).toFixed(1);
      _inFlight[cmd]--;
      if (!quiet) console.debug(`[IPC] ✓ ${cmd} ${ms}ms (in-flight: ${_inFlight[cmd]})`);
      return result;
    })
    .catch(err => {
      const ms = (performance.now() - t0).toFixed(1);
      _inFlight[cmd]--;
      console.error(`[IPC] ✗ ${cmd} ${ms}ms err:`, err);
      throw err;
    });
}

// ── Types ────────────────────────────────────────────────────────────────────

export interface LlmConfig {
  id: string; name: string; model: string;
  endpoint: string; api_key: string; ctx_window: string;
  temperature: number; enabled: boolean; api_spec: ApiSpec;
  supports_vision: boolean; created_at: string;
}
/** 接口规范（wire spec）：决定文本生成路由与工具调用格式。目前仅支持两种。 */
export type ApiSpec = 'openai' | 'anthropic';
export interface Agent {
  id: string; name: string; name_en: string; role: string;
  color: string; initial: string; llm_id: string | null;
  system_prompt: string; forge_role: string | null;
  role_type: 'business' | 'system'; system_kind: string | null;
  capabilities_json: string; max_concurrency: number;
  visible_in_chat: boolean; mentionable: boolean; enabled: boolean;
  prompt_mode: 'builtin' | 'append' | 'custom';
  memory_enabled: boolean;
  created_at: string;
}
export interface RoleSlot {
  kind: string; name: string; name_en: string;
  group: 'orchestration' | 'delivery' | 'pipeline' | 'knowledge';
  binding: 'system_kind' | 'forge_role';
  desc: string; usage: string; color: string; icon: string;
  builtin_prompt: string;
  llm_only: boolean;
  holder: Agent | null;
}
export interface Project {
  id: string; name: string; slug: string; description: string;
  repo_path: string; branch_dev: string; branch_main: string;
  status: string; config_yaml: string | null;
  is_default: boolean;
  archived_at?: string | null;
  code_agent_id?: string | null;
  created_at: string; updated_at: string;
}
export interface Issue {
  id: string; project_id: string; source_type: string;
  title: string; description: string; category: string;
  severity: string; priority: number | null; status: string;
  fingerprint: string; created_at: string; updated_at: string;
  source_ref?: string | null; raw_capture?: string | null;
  repro_steps?: string | null; environment?: string | null;
  expected?: string | null; actual?: string | null;
  acceptance_json?: string | null;
  /** 1 = 该需求由「恢复需求」从已撤销态重回队列（用于队列里的小 dot 标识）。 */
  restored_from_revert?: number;
}
// 需求来源（issues.source_type）→ 人类可读标签 + 语义 chip 色。
// 来源真源是后端落库的 source_type 字面量；新增来源时在此登记即可全局生效。
export const ISSUE_SOURCE_META: Record<string, { label: string; chip: string }> = {
  manual:         { label: '手动提交',      chip: '' },
  bulk:           { label: '批量导入',      chip: '' },
  github:         { label: 'GitHub 接入',   chip: 'blue' },
  webhook:        { label: 'Webhook',       chip: 'blue' },
  conversation:   { label: '会议室',        chip: '' },
  quickcapture:   { label: '速录',          chip: '' },
  meeting:        { label: '会议录音',      chip: '' },
  todo_scan:      { label: '自动供料 · 扫描', chip: 'violet' },
  code_analysis:  { label: '自动供料 · 分析', chip: 'violet' },
  proposer:       { label: '自动供料 · 提议', chip: 'violet' },
  security_audit: { label: '安全审计',      chip: 'amber' },
};
/** 取需求来源的展示元信息；未登记的来源原样回显，避免丢失信息。 */
export function issueSourceMeta(src?: string | null): { label: string; chip: string } {
  return ISSUE_SOURCE_META[src ?? ''] ?? { label: src || '未知来源', chip: '' };
}

export interface IssueAnalysis {
  id: string; issue_id: string; authenticity_score: number;
  feasibility_score: number | null; priority_suggestion: number | null;
  category_suggestion: string | null; severity_suggestion: string | null;
  duplicate_of: string | null; affected_modules: string | null;
  analysis_summary: string; raw_llm_output: string | null;
  analysis_json: string | null; created_at: string;
}

// 完整结构化分析规格（对齐 src-tauri/src/agents/schemas/issue_analysis.schema.json v1.0）
export interface IssueAnalysisSpec {
  schema_version: string;
  triage: {
    authenticity_score: number; feasibility_score: number; priority_suggestion: number;
    category_suggestion: string; severity_suggestion: string; is_duplicate: boolean;
    duplicate_of: string | null; duplicate_hint: string;
    /** 分析判定该需求是否真的需要改动代码；false 表示误报/已实现/纯提问，AutoForge 默认不建议执行。缺省视为 true。 */
    needs_changes?: boolean; no_change_reason?: string;
    analysis_summary: string; confidence: number;
  };
  understanding: {
    problem_type: string; restated_issue: string; restated_requirement?: string; user_story: string | null;
    current_behavior: string | null; expected_behavior: string | null;
    reproduction_steps: string[]; background: string | null;
  };
  root_cause: {
    hypothesis: string; evidence: string[];
    suspected_locations: { file: string; symbol: string | null; reason: string }[];
    confidence: number;
  } | null;
  scope: {
    affected_modules: string[];
    affected_files: { path: string; change_type: string; reason: string; confidence?: number | null }[];
    related_files: string[]; entry_points: string[]; out_of_scope: string[]; blast_radius: string;
  };
  implementation_plan: {
    approach: string;
    steps: { order: number; action: string; target_files: string[]; details: string | null }[];
    alternatives: { option: string; tradeoff: string }[];
    data_model_changes: { kind: string; description: string }[];
    new_dependencies: string[];
  };
  acceptance_criteria: { id: string; statement: string; verify: string | null }[];
  test_plan: { unit: string[]; integration: string[]; manual: string[]; edge_cases: string[] };
  risks: { description: string; severity: string; mitigation: string | null }[];
  constraints: { must: string[]; must_not: string[] };
  assumptions: string[];
  open_questions: string[];
  estimate: { complexity: string; confidence: number | null; rationale: string | null } | null;
  claude_code_brief: {
    objective: string; instructions: string[]; do: string[]; dont: string[];
    files_to_touch: string[]; definition_of_done: string[];
  };
}

export function parseAnalysisSpec(json: string | null | undefined): IssueAnalysisSpec | null {
  if (!json) return null;
  try {
    const v = JSON.parse(json);
    // 分析失败时可能落 "{}" / null / 数组等占位；非「含实际内容的对象」一律视为无 spec，
    // 让调用方回退到简单视图，避免下游组件在缺失字段上解引用而崩溃。
    if (!v || typeof v !== 'object' || Array.isArray(v) || Object.keys(v).length === 0) return null;
    return v as IssueAnalysisSpec;
  } catch { return null; }
}
export interface ChangeRequest {
  id: string; project_id: string; issue_id: string; status: string;
  admin_id: string | null; approved_at: string | null;
  admin_suggestions_1: string | null; admin_suggestions_2: string | null;
  merge_commit_message: string | null;
  target_branch: string; created_at: string; updated_at: string;
}
export interface WorktreeSession {
  id: string; change_request_id: string; worktree_path: string;
  branch_name: string; status: string; prompt_snapshot: string | null;
  iteration_count: number; report_content: string | null;
  /** 合并到 dev 时产生的 squash 提交 SHA；为 null 时无法一键撤销（合并早于撤销功能）。 */
  merge_commit: string | null;
  started_at: string | null; completed_at: string | null;
}
/** 代码 Agent 单次执行日志的元信息（列表用，不含正文）。 */
export interface CodeAgentRunMeta {
  id: string; change_request_id: string; worktree_session_id: string | null;
  phase: string; kind: string; model: string | null;
  exit_code: number; duration_ms: number;
  stdout_bytes: number; stderr_bytes: number; truncated: number;
  created_at: string;
}
/** 完整一条执行日志（含 stdout/stderr 正文）。 */
export interface CodeAgentRunLog extends CodeAgentRunMeta {
  stdout: string; stderr: string;
}
export interface Conversation {
  id: string; conv_type: string; name: string | null;
  color: string; initial: string | null; created_at: string;
  members: string[]; unread: number;
  last_message: string | null; last_time: string | null;
  project_id: string | null;
}
export interface ProjectContextFile {
  rel_path: string; name: string; size_bytes: number;
  is_priority: boolean; pinned: boolean;
}
export interface Message {
  id: string; conversation_id: string; from_agent: string | null;
  content_json: string; created_at: string;
  excluded_from_context: boolean;
}
export interface ConversationTask {
  id: string; conversation_id: string; trigger_message_id: string;
  planner_agent_id: string | null; instruction: string; plan_json: string;
  status: string; error: string | null; created_at: string;
  started_at: string | null; completed_at: string | null;
}
export interface ConversationAttachment {
  id: string; conversation_id: string; original_name: string;
  stored_name: string; rel_path: string; mime: string; kind: string;
  size_bytes: number; sha256: string; created_at: string;
}
export interface PipelineStats {
  triage: number;
  pending_analysis: number; pending_review_1: number;
  executing: number; pending_review_2: number;
  merged: number; rejected: number; total_issues: number;
  active_projects: number;
  active_slots: number; max_slots: number; total_slot_capacity: number;
  pending_review_slots: number; pause_threshold: number;
  stage: string; executing_cr_ids: string[];
  project_slots: Array<{
    project_id: string; project_name: string; project_status: string;
    active_slots: number; max_slots: number;
    executing_slots: number; pending_review_slots: number;
    occupants: Array<{ id: string; status: string }>;
  }>;
  project_pipelines: Array<{
    project_id: string; project_name: string; project_status: string;
    triage: number;
    pending_analysis: number; pending_review_1: number;
    executing: number; pending_review_2: number;
    merged: number; rejected: number; total_issues: number;
  }>;
}
export interface SystemHealth {
  status: string; claude_auth: boolean; db_ok: boolean;
  version: string; active_slots: number; max_slots: number; total_slot_capacity: number;
  pending_review: number; pause_threshold: number; stage: string;
}
export interface ConcurrencyConfig {
  active_slots: number; max_slots: number; pending_review: number;
  pause_threshold: number; stage: string; queue_strategy: string;
  timeout_min: number; idle_timeout_min: number; max_load_factor: number;
  build_slots: number; cpu_budget_pct: number;
}
export interface PreviewEnvironment {
  id: string; project_id: string; env_type: string;
  worktree_session_id: string | null; container_id: string | null;
  preview_url: string; db_snapshot_name: string | null;
  status: string; data_masked_at: string | null;
  mask_policy_version: string | null; created_at: string;
  ready_at: string | null; terminated_at: string | null;
}
export interface TestSession {
  id: string; project_id: string; session_type: string;
  change_request_id: string | null; trigger: string; status: string;
  summary: string; results_json: string; issues_created: string;
  started_at: string | null; completed_at: string | null;
}
export interface ScanFinding {
  id: string; test_session_id: string; check_type: string;
  severity: string; title: string; description: string;
  file_path: string | null; line_number: number | null;
  fingerprint: string; issue_entry_id: string | null; created_at: string;
}
export interface AdminDecision {
  id: string; project_id: string; issue_id: string;
  change_request_id: string | null; stage: string; decision: string;
  admin_id: string; suggestions: string | null; created_at: string;
}

export interface BadgeCounts { chat_unread: number; audit_pending: number; }

export interface SystemResources {
  cpu_pct: number; mem_pct: number; mem_used_mb: number; mem_total_mb: number;
}

export interface DevServerStatus {
  project_id: string;
  status: 'no_config' | 'idle' | 'starting' | 'running' | 'stopped';
  url: string | null;
}

// ── System ───────────────────────────────────────────────────────────────────
export const getSystemHealth = () => ipc<SystemHealth>('system_health');
export const getSystemResources = () => ipc<SystemResources>('system_resources');
export const checkClaudeAuth = () => ipc<boolean>('check_claude_auth');
export const getPipelineStats = () => ipc<PipelineStats>('pipeline_stats');
export const getBadgeCounts = () => ipc<BadgeCounts>('get_badge_counts');
export const getConcurrencyConfig = () => ipc<ConcurrencyConfig>('get_concurrency_config');
export const updateConcurrencyConfig = (payload: Partial<{
  max_slots: number; pause_threshold: number; queue_strategy: string;
  timeout_min: number; idle_timeout_min: number; max_load_factor: number;
  build_slots: number; cpu_budget_pct: number;
}>) => ipc<ConcurrencyConfig>('update_concurrency_config', { payload });
export const listPreviewEnvironments = (projectId?: string, status?: string) =>
  ipc<PreviewEnvironment[]>('list_preview_environments', {
    projectId: projectId ?? null, status: status ?? null,
  });
export const listTestSessions = (projectId?: string) =>
  ipc<TestSession[]>('list_test_sessions', { projectId: projectId ?? null });
export const listScanFindings = (testSessionId?: string) =>
  ipc<ScanFinding[]>('list_scan_findings', { testSessionId: testSessionId ?? null });
export const listAdminDecisions = (projectId?: string) =>
  ipc<AdminDecision[]>('list_admin_decisions', { projectId: projectId ?? null });

// ── Code Agents（可插拔代码实现 agent：claude / codex / opencode） ──────────────
export interface CodeAgent {
  id: string; kind: string; label: string; program: string;
  model: string | null; fast_model: string | null; strong_model: string | null;
  extra_args_json: string;
  enabled: boolean; is_default: boolean; created_at: string;
}
export interface UpsertCodeAgentPayload {
  id?: string | null; kind: string; label: string; program: string;
  model?: string | null; fast_model?: string | null; strong_model?: string | null;
  extra_args?: string[]; enabled?: boolean;
}
export const listCodeAgents = () => ipc<CodeAgent[]>('list_code_agents');
export const upsertCodeAgent = (payload: UpsertCodeAgentPayload) =>
  ipc<CodeAgent>('upsert_code_agent', { payload });
export const deleteCodeAgent = (id: string) => ipc<void>('delete_code_agent', { id });
export const setDefaultCodeAgent = (id: string) => ipc<void>('set_default_code_agent', { id });
export const setProjectCodeAgent = (projectId: string, codeAgentId: string | null) =>
  ipc<void>('set_project_code_agent', { projectId, codeAgentId });
// 代码 Agent 可用性探测结果：工具是否就绪 + （probeModel 时）当前配置模型的探测结论。
export interface CodeAgentProbe {
  tool: boolean;                  // CLI 已安装并（可探测时）登录
  model: boolean | null;          // 模型探测结论：null=未探测/未配置模型；true/false=可用/不可用
  model_name: string;             // 被探测的模型名（未配置/未探测为空）
  detail: string;                 // 失败原因或说明（成功为空）
}
// probeModel=true 时额外发极小 prompt 验证配置的模型（慢几秒、烧极少量 token）；
// 进页面自动检测传 false（仅工具探测，轻量）。
export const checkCodeAgentAuth = (id: string, probeModel = false) =>
  ipc<CodeAgentProbe>('check_code_agent_auth', { id, probeModel });

// 编码 Agent 技能（Skill）：注入到 worktree 供编码 agent 消费的可复用指令包。
// claude 写 .claude/skills 走原生渐进披露；codex/opencode 折叠进 prompt。
export interface CodeAgentSkill {
  id: string; name: string; description: string; body: string;
  project_id: string | null; enabled: boolean; created_at: string; updated_at: string;
}
export interface UpsertCodeAgentSkillPayload {
  id?: string | null; name: string; description: string; body: string;
  project_id?: string | null; enabled?: boolean;
}
export const listCodeAgentSkills = () => ipc<CodeAgentSkill[]>('list_code_agent_skills');
export const upsertCodeAgentSkill = (payload: UpsertCodeAgentSkillPayload) =>
  ipc<CodeAgentSkill>('upsert_code_agent_skill', { payload });
export const deleteCodeAgentSkill = (id: string) =>
  ipc<void>('delete_code_agent_skill', { id });

// 错误历史：后端后台任务失败记录（runner 持久化的 last_error），供设置页排错。
export interface JobFailure {
  id: string;
  job_type: string;
  status: string;
  attempt: number;
  last_error?: string | null;
  updated_at: string;
}
export const listJobFailures = (limit?: number) =>
  ipc<JobFailure[]>('list_job_failures', { limit: limit ?? null });

// ── Self-update (AutoForge managing its own running repo) ─────────────────────
export interface SelfUpdateStatus {
  repo_path: string;
  branch: string;
  is_self_managed: boolean;
  dirty: boolean;
  behind: number;
  ahead: number;
}
export interface SelfUpdateResult {
  ok: boolean;
  pulled: number;
  message: string;
  restart_required: boolean;
}
export interface SelfUpdatePending {
  project_id: string | null;
  behind: number;
}
export const selfUpdatePending = () =>
  ipc<SelfUpdatePending>('self_update_pending');
export const selfUpdateStatus = (projectId: string) =>
  ipc<SelfUpdateStatus>('self_update_status', { projectId });
export const selfUpdatePull = (projectId: string) =>
  ipc<SelfUpdateResult>('self_update_pull', { projectId });

// ── Projects ─────────────────────────────────────────────────────────────────
export const listProjects = () => ipc<Project[]>('list_projects');
export const listActiveProjects = () => ipc<Project[]>('list_active_projects');
export const getProject = (id: string) => ipc<Project>('get_project', { id });
export const createProject = (payload: {
  name: string; slug: string; description?: string;
  repo_path: string; branch_dev?: string; branch_main?: string;
  config_yaml?: string;
}) => ipc<Project>('create_project', { payload });
export const createLocalProject = (payload: {
  name: string; slug: string; description?: string;
  repo_path: string; branch_dev?: string; branch_main?: string;
  config_yaml?: string;
}) => ipc<Project>('create_local_project', { payload });
export const cloneProjectFromGit = (payload: {
  name: string; slug: string; description?: string;
  git_url: string; target_path: string; clone_branch?: string;
  git_username?: string; git_password?: string;
  branch_dev?: string; branch_main?: string; config_yaml?: string;
}) => ipc<Project>('clone_project_from_git', { payload });
export const updateProject = (id: string, payload: Partial<{
  name: string; description: string; repo_path: string;
  branch_dev: string; branch_main: string; status: string; config_yaml: string;
}>) => ipc<Project>('update_project', { id, payload });
/** 设为默认项目（全局唯一，其他页面置顶并优先选中）。传空串清除默认。 */
export const setDefaultProject = (id: string) => ipc<void>('set_default_project', { id });
/** 软删除：归档项目（保留 DB 数据，重新添加同仓库时按 .autoforge/project.json 身份锚挂回）。 */
export const deleteProject = (id: string) => ipc<void>('delete_project', { id });
/** 回收站：列出已归档项目。 */
export const listArchivedProjects = () => ipc<Project[]>('list_archived_projects');
/** 从回收站恢复一个已归档项目。 */
export const restoreProject = (id: string) => ipc<Project>('restore_project', { id });
/** 彻底删除（不可恢复）：级联清除该项目全部 DB 数据。 */
export const purgeProject = (id: string) => ipc<void>('purge_project', { id });

/** AI 推断的运行配置草稿（字段对齐 ProjectConfigForm，全部可选）。 */
export interface RunConfigDraft {
  dev_kind?: string | null;
  dev_command?: string | null;
  app_command?: string | null;
  test_unit?: string | null;
  test_unit_timeout?: number | null;
  test_integration?: string | null;
  test_integration_timeout?: number | null;
  quality_lint?: string | null;
  quality_typing?: string | null;
  quality_security?: string | null;
  project_language?: string | null;
  project_framework?: string | null;
  preview_build?: string | null;
  preview_start?: string | null;
  deploy_command?: string | null;
}
export const aiGenerateRunConfig = (projectId: string) =>
  ipc<RunConfigDraft>('ai_generate_run_config', { projectId });

// ── Issues ───────────────────────────────────────────────────────────────────
export const listIssues = (projectId?: string) =>
  ipc<Issue[]>('list_issues', { projectId: projectId ?? null });
// 分页查询需求（总账滚动动态加载）：按状态/关键字过滤，updated_at 倒序，返回当前页 + 总数。
export interface IssuePage { items: Issue[]; total: number }
export const listIssuesPage = (
  projectId: string | undefined,
  status: string | undefined,
  search: string | undefined,
  limit: number,
  offset: number,
  excludeMerged?: boolean,   // 与功能审计页「显示已合并需求」开关共享：true 时隐藏已合并需求
  sortAsc?: boolean,         // 按创建时间排序方向：true=正序（最早在前，默认），false=倒序
) => ipc<IssuePage>('list_issues_page', {
  projectId: projectId || null,
  status: status && status !== 'all' ? status : null,
  search: search?.trim() || null,
  excludeMerged: excludeMerged ?? false,
  limit, offset,
  sortAsc: sortAsc ?? true,
});
// 某项目出现过的全部需求状态（去重），用于总账筛选 chip。
export const listIssueStatuses = (projectId: string) =>
  ipc<string[]>('list_issue_statuses', { projectId });
// 按状态集合取需求（有界子集，如需求审核 队列），替代全量加载。
export const listIssuesByStatuses = (projectId: string, statuses: string[]) =>
  ipc<Issue[]>('list_issues_by_statuses', { projectId, statuses });
// 批量取需求标题（轻量），用于变更请求列表解析标题。
export const listIssueTitles = (ids: string[]) =>
  ipc<{ id: string; title: string }[]>('list_issue_titles', { ids });
export const getIssue = (id: string) => ipc<Issue>('get_issue', { id });
export const getIssueAnalysis = (issueId: string) =>
  ipc<IssueAnalysis | null>('get_issue_analysis', { issueId });
export const submitIssue = (payload: {
  project_id: string; title: string; description?: string;
  category?: string; severity?: string; source_type?: string;
  mode?: 'flow' | 'triage';
  repro_steps?: string; environment?: string; expected?: string; actual?: string;
}) => ipc<Issue>('submit_issue', { payload });
// 分析失败/卡住时重新触发需求分析（回到分析队列）。
export const retryAnalysis = (issueId: string) =>
  ipc<void>('retry_analysis', { issueId });

// 需求审核 补充意见重评：带管理员补充意见重新分析当前需求，之后重回需求审核。
export const reanalyzeWithFeedback = (issueId: string, feedback: string) =>
  ipc<void>('reanalyze_with_feedback', { issueId, feedback });

// 人审改 AI 生成的验收标准（acceptance_json，JSON 数组字符串）。
export const updateIssueAcceptance = (issueId: string, acceptanceJson: string) =>
  ipc<void>('update_issue_acceptance', { issueId, acceptanceJson });

// ── 需求附件（图片/文档；图片可喂给 supports_vision 的分析 Agent）─────────────
export interface IssueAttachment {
  id: string; issue_id: string; original_name: string;
  stored_name: string; rel_path: string; mime: string; kind: string;
  size_bytes: number; sha256: string; created_at: string;
}
export const importIssueAttachment = (payload: {
  issue_id: string; file_name: string; mime_hint: string; data_base64: string;
}) => ipc<IssueAttachment>('import_issue_attachment', { payload });
export const listIssueAttachments = (issueId: string) =>
  ipc<IssueAttachment[]>('list_issue_attachments', { issueId });
export const issueAttachmentDataUrl = (attachmentId: string) =>
  ipc<string>('issue_attachment_data_url', { attachmentId });
export const openIssueAttachment = (attachmentId: string) =>
  ipc<void>('open_issue_attachment', { attachmentId });
export const deleteIssueAttachment = (attachmentId: string) =>
  ipc<void>('delete_issue_attachment', { attachmentId });

// ── 项目蓝图向导（大需求 → PRD / 技术规格 / TASKLIST）─────────────────────────
export interface BlueprintSpec { category: string; title: string; content_markdown: string; }
export interface BlueprintTask { title: string; description: string; category: string; severity: string; }
export interface BlueprintResult {
  prd_markdown: string; specs: BlueprintSpec[]; tasklist: BlueprintTask[]; tasklist_markdown: string;
}
export const generateProjectBlueprint = (projectId: string, brief: string) =>
  ipc<BlueprintResult>('generate_project_blueprint', { projectId, brief });
export const applyProjectBlueprint = (
  projectId: string, prdMarkdown: string, specs: BlueprintSpec[], tasklistMarkdown?: string | null,
) => ipc<string>('apply_project_blueprint', {
  projectId, prdMarkdown, specs, tasklistMarkdown: tasklistMarkdown ?? null,
});
export const enqueueBlueprintTasks = (projectId: string, tasks: BlueprintTask[]) =>
  ipc<number>('enqueue_blueprint_tasks', { projectId, tasks });

// CR 级测试遥测记录。
export interface CrTestRun {
  id: string; cr_id: string; result: string; summary: string; run_by: string; run_at: string;
}
export const listCrTestRuns = (crId: string) =>
  ipc<CrTestRun[]>('list_cr_test_runs', { crId });

// ── Change Requests ──────────────────────────────────────────────────────────
export const listChangeRequests = (projectId?: string, status?: string) =>
  ipc<ChangeRequest[]>('list_change_requests', {
    projectId: projectId ?? null, status: status ?? null,
  });
// 分页查询变更请求（功能审计代码闸滚动加载）：按状态过滤，created_at 倒序，返回当前页 + 总数。
// 活动集（非合并）一次取够（大 limit）；已合并历史按页滚动加载，total 供「已合并」徽标计数。
export interface ChangeRequestPage { items: ChangeRequest[]; total: number }
export const listChangeRequestsPage = (
  projectId: string | undefined,
  status: string | undefined,
  excludeMerged: boolean,
  limit: number,
  offset: number,
) => ipc<ChangeRequestPage>('list_change_requests_page', {
  projectId: projectId || null,
  status: status && status !== 'all' ? status : null,
  excludeMerged,
  limit, offset,
});
export const getChangeRequest = (id: string) =>
  ipc<ChangeRequest>('get_change_request', { id });
// 按需求 id 取其最新 CR：左栏分批加载后从总账下钻到未载入的 CR 时按需补拉单条。
export const getChangeRequestByIssue = (issueId: string) =>
  ipc<ChangeRequest | null>('get_change_request_by_issue', { issueId });
export const getWorktreeSession = (crId: string) =>
  ipc<WorktreeSession | null>('get_worktree_session', { crId });
// 代码 Agent 执行日志：列表只拿元信息，详情按 id 拉完整 stdout/stderr。
export const listCodeAgentRuns = (crId: string) =>
  ipc<CodeAgentRunMeta[]>('list_code_agent_runs', { crId });
export const getCodeAgentRun = (id: string) =>
  ipc<CodeAgentRunLog | null>('get_code_agent_run', { id });
// 运行中编码 Agent 的实时日志快照：中途进入「执行日志」时回灌已错过的开头。
// next_seq 为下一个 chunk 序号，前端据此与增量事件去重无缝续接。
export const getRunningCodeAgentLog = (crId: string) =>
  ipc<{ text: string; next_seq: number }>('get_running_code_agent_log', { crId });
export const getCodeDiff = (crId: string) =>
  ipc<string>('get_code_diff', { crId });

// AI 变更摘要：基于 CR 的代码 diff 生成结构化摘要（文件清单 + 业务意图分类 + 敏感标签）。
// 后端按 cr_id + diff 哈希缓存：切换需求/CR 命中缓存直接返回，不重跑 LLM；diff 变化哈希自动失效重算。
// force=true（卡片「重新生成」）跳过缓存强制重算；LLM 失败时降级返回确定性部分（status='degraded'）。
export interface SummaryFile { path: string; change: 'added' | 'modified' | 'deleted' | 'renamed'; note: string }
export interface IntentGroup { kind: string; detail: string }
export interface SensitiveTag { kind: string; label: string; detail: string }
export interface ChangeSummary {
  overview: string;
  files: SummaryFile[];
  intents: IntentGroup[];
  sensitive: SensitiveTag[];
  status: 'ok' | 'degraded' | 'empty';
  note: string | null;
}
export const generateChangeSummary = (crId: string, force = false) =>
  ipc<ChangeSummary>('generate_change_summary', { crId, force });
export type MergeConflictView = { files: string[]; diff: string };
export const getMergeConflict = (crId: string) =>
  ipc<MergeConflictView>('get_merge_conflict', { crId });
export const retryMerge = (crId: string) =>
  ipc<void>('retry_merge', { crId });
export const aiResolveMergeConflict = (crId: string) =>
  ipc<void>('ai_resolve_merge_conflict', { crId });
/** 撤销一个已合并需求的改动（在 dev 上 git revert 其 squash 提交）。 */
export const revertChangeRequest = (crId: string) =>
  ipc<void>('revert_change_request', { crId });
/** 恢复一个已撤销的需求：重新进入执行队列，再次实现并经过代码审核后合并。 */
export const restoreChangeRequest = (crId: string) =>
  ipc<ChangeRequest>('restore_change_request', { crId });

// ── 人工解冲突（方案 B 逐 hunk 决策 + C 外部 IDE 兜底）────────────────────────
export type ConflictSegment = {
  kind: 'context' | 'conflict';
  text: string | null; ours: string | null; theirs: string | null; base: string | null;
};
export type ConflictFile = { path: string; binary: boolean; segments: ConflictSegment[] };
export type ConflictDetail = { files: ConflictFile[]; resolvable: boolean };
/** 读取 CR 的冲突现场（三方有序段），后端按需物化冲突态。 */
export const getConflictDetail = (crId: string) =>
  ipc<ConflictDetail>('get_conflict_detail', { crId });
/** 人工解决冲突：files 为 path→整文件内容（应用内）；传 null 表示已在外部 IDE 改好盘。 */
export const resolveConflictManually = (crId: string, files: Record<string, string> | null) =>
  ipc<void>('resolve_conflict_manually', { crId, files });
/** 外部 IDE 兜底：物化冲突态并在文件管理器中定位 worktree，返回其路径。 */
export const openConflictWorkspace = (crId: string) =>
  ipc<string>('open_conflict_workspace', { crId });
export const review1 = (issueId: string, decision: {
  decision: string; suggestions?: string; admin_id?: string;
}) => ipc<ChangeRequest>('review_1', { issueId, decision });
// 批量需求审核：一次性通过多条待需求审核的需求，快速清空需求审核 队列。
// 非 pending_issue_review 的 id 会被跳过（skipped）而非报错，避免选区过期导致整批失败。
export interface Review1BatchResult { approved: number; skipped: number; errors: number; }
export const review1Batch = (issueIds: string[], suggestions?: string) =>
  ipc<Review1BatchResult>('review_1_batch', { issueIds, suggestions: suggestions || undefined });

// ── 同文件多需求合并 ───────────────────────────────────────────────────────────
// 合并候选组：命中同一实质文件、可合并成一次变更的待需求审核需求。
export interface MergeCandidate {
  issue_ids: string[];
  titles: string[];
  shared_files: string[];
  total_files: number;
  strength: 'strong' | 'weak';
  conflict_hint: string | null;
}
/** 列出某项目需求审核队列里的合并候选组（按共享实质文件聚类，已剔除枢纽文件）。 */
export const listMergeCandidates = (projectId: string) =>
  ipc<MergeCandidate[]>('list_merge_candidates', { projectId });
/** 合并需求审核：把多条需求合并成一个 CR + 一次执行；primaryId 缺省取第一条。 */
export const review1Merge = (
  issueIds: string[],
  primaryId?: string,
  suggestions?: string,
) =>
  ipc<ChangeRequest>('review_1_merge', {
    issueIds,
    primaryId: primaryId || undefined,
    suggestions: suggestions || undefined,
  });
/** 从尚未进入执行的合并 CR 中拆出某成员需求，退回独立审核。 */
export const splitChangeRequest = (crId: string, issueId: string) =>
  ipc<void>('split_change_request', { crId, issueId });
// 一个 CR 覆盖的需求引用（合并 CR 含多条；普通 CR 一条）。
export interface CrIssueRef { issue_id: string; title: string; role: string; status: string; }
/** 列出某 CR 覆盖的全部需求（primary 在前），供「覆盖 N 个需求」展示。 */
export const getChangeRequestIssues = (crId: string) =>
  ipc<CrIssueRef[]>('get_change_request_issues', { crId });

export const review2 = (crId: string, decision: {
  decision: string; suggestions?: string; admin_id?: string; commit_message?: string;
}) => ipc<ChangeRequest>('review_2', { crId, decision });
// 批量代码审核：一次性通过多条待代码审核 的变更请求，各自排队合并，快速清空代码审核 队列。
// 非 pending_code_review 的 id 会被跳过（skipped）而非报错，避免选区过期导致整批失败。
export interface Review2BatchResult { approved: number; skipped: number; errors: number; }
export const review2Batch = (crIds: string[], suggestions?: string) =>
  ipc<Review2BatchResult>('review_2_batch', { crIds, suggestions: suggestions || undefined });
export const retryChangeRequest = (crId: string) =>
  ipc<ChangeRequest>('retry_change_request', { crId });
export const deleteChangeRequest = (crId: string) =>
  ipc<void>('delete_change_request', { crId });

// ── Conversations ────────────────────────────────────────────────────────────
export const listConversations = () => ipc<Conversation[]>('list_conversations');
export const listMessages = (conversationId: string) =>
  ipc<Message[]>('list_messages', { conversationId });
export const sendMessage = (payload: { conversation_id: string; content_json: string }) =>
  ipc<Message>('send_message', { payload });
export const importAttachment = (payload: {
  conversation_id: string; file_name: string; mime_hint: string; data_base64: string;
}) => ipc<ConversationAttachment>('import_attachment', { payload });
export const listConversationAttachments = (conversationId: string) =>
  ipc<ConversationAttachment[]>('list_conversation_attachments', { conversationId });
export const openAttachment = (attachmentId: string) =>
  ipc<void>('open_attachment', { attachmentId });
export const attachmentDataUrl = (attachmentId: string) =>
  ipc<string>('attachment_data_url', { attachmentId });
export const createGroupConversation = (
  name: string, memberIds: string[], color?: string, initial?: string, projectId?: string | null,
) => ipc<Conversation>('create_group_conversation', { name, memberIds, color, initial, projectId: projectId ?? null });
export const updateGroupConversation = (conversationId: string, name: string, projectId?: string | null) =>
  ipc<Conversation>('update_group_conversation', { conversationId, name, projectId: projectId ?? null });
export const addConversationMember = (conversationId: string, agentId: string) =>
  ipc<Conversation>('add_conversation_member', { conversationId, agentId });
export const removeConversationMember = (conversationId: string, agentId: string) =>
  ipc<Conversation>('remove_conversation_member', { conversationId, agentId });
export const deleteGroupConversation = (conversationId: string) =>
  ipc<void>('delete_group_conversation', { conversationId });
export const clearConversationMessages = (conversationId: string) =>
  ipc<void>('clear_conversation_messages', { conversationId });

// ── 会议室对话归档（只读快照 + 检索回顾） ─────────────────────────────────────
export interface ArchivedMessage {
  from_agent: string | null;
  author: string;
  author_en: string;
  author_color: string;
  is_me: boolean;
  is_innate: boolean;
  content_json: string;
  created_at: string;
  excluded_from_context: boolean;
}
export interface ConversationArchiveSummary {
  id: string; conversation_id: string; conv_type: string; title: string;
  project_id: string | null; project_name: string | null;
  message_count: number; archived_at: string;
}
export interface ConversationArchiveDetail extends ConversationArchiveSummary {
  payload_json: string;
}
export interface ArchiveSearchHit extends ConversationArchiveSummary {
  match_count: number; snippet: string;
}
export const archiveConversation = (conversationId: string) =>
  ipc<ConversationArchiveSummary>('archive_conversation', { conversationId });
export const listConversationArchives = () =>
  ipc<ConversationArchiveSummary[]>('list_conversation_archives');
export const getConversationArchive = (archiveId: string) =>
  ipc<ConversationArchiveDetail>('get_conversation_archive', { archiveId });
export const searchConversationArchives = (query: string) =>
  ipc<ArchiveSearchHit[]>('search_conversation_archives', { query });
export const deleteConversationArchive = (archiveId: string) =>
  ipc<void>('delete_conversation_archive', { archiveId });

export const markConversationRead = (conversationId: string) =>
  ipc<void>('mark_conversation_read', { conversationId });
export const agentReply = (conversationId: string, agentId: string, windowSize?: number) =>
  ipc<void>('agent_reply', { conversationId, agentId, windowSize: windowSize ?? null });
export const toggleMessageContext = (messageId: string) =>
  ipc<Message>('toggle_message_context', { messageId });
export const startConversationTask = (payload: {
  conversation_id: string; trigger_message_id: string; instruction: string;
  mentioned_agent_ids: string[]; window_size?: number | null;
}) => ipc<ConversationTask>('start_conversation_task', { payload });
export const listConversationTasks = (conversationId: string) =>
  ipc<ConversationTask[]>('list_conversation_tasks', { conversationId });
// 会议室「立即编码」：① AI 起草功能点工单草稿；② 据确认工单直奔编码（建需求→CR→执行，跳过需求审核队列）。
export const draftCodingBrief = (conversationId: string, windowSize?: number) =>
  ipc<string>('draft_coding_brief', { conversationId, windowSize: windowSize ?? null });
export const startConversationCoding = (payload: {
  conversation_id: string; title: string; brief: string; window_size?: number | null;
}) => ipc<ChangeRequest>('start_conversation_coding', { payload });
// 总结/结论 + 压缩上下文：生成摘要并把当前窗口内历史消息移出后续上下文。
export const compressConversationContext = (payload: {
  conversation_id: string; mode: 'summary' | 'conclusion';
}) => ipc<void>('compress_conversation_context', { payload });

// ── Innate 知识库（会议室命令 + 自成长配置） ─────────────────────────────────
export const INNATE_SENDER = '__innate__';
export type ConvCommandName = 'remember' | 'recall' | 'evolve' | 'innate';
export const runConversationCommand = (payload: {
  conversation_id: string; command: ConvCommandName; arg: string;
}) => ipc<Message>('run_conversation_command', { payload });
export interface KnowledgeSettings { evolve_interval_hours: number; capture_threshold: number; }
export const getKnowledgeSettings = () =>
  ipc<KnowledgeSettings>('get_knowledge_settings');
export const setKnowledgeSettings = (payload: KnowledgeSettings) =>
  ipc<KnowledgeSettings>('set_knowledge_settings', { payload });

export interface EmbeddingSettings {
  provider: string; base_url: string; model_id: string; api_key: string; dim: number;
}
export const getKnowledgeEmbedding = () =>
  ipc<EmbeddingSettings>('get_knowledge_embedding');
export const setKnowledgeEmbedding = (payload: EmbeddingSettings) =>
  ipc<EmbeddingSettings>('set_knowledge_embedding', { payload });
export const listProjectFiles = (projectId: string, conversationId?: string) =>
  ipc<ProjectContextFile[]>('list_project_files', { projectId, conversationId: conversationId ?? null });
export const readProjectFile = (projectId: string, relPath: string) =>
  ipc<string>('read_project_file', { projectId, relPath });
export const listConversationProjectContext = (conversationId: string) =>
  ipc<string[]>('list_conversation_project_context', { conversationId });
export const addConversationProjectContext = (conversationId: string, relPath: string) =>
  ipc<void>('add_conversation_project_context', { conversationId, relPath });
export const removeConversationProjectContext = (conversationId: string, relPath: string) =>
  ipc<void>('remove_conversation_project_context', { conversationId, relPath });

// ── Workspace (.autoforge) ───────────────────────────────────────────────────
export interface WorkspaceFile {
  rel_path: string; name: string; subfolder: string;
  size_bytes: number; modified_at: string;
}
export const ensureWorkspaceDirs = (projectId: string) =>
  ipc<void>('ensure_workspace_dirs', { projectId });
export const listWorkspaceFiles = (projectId: string) =>
  ipc<WorkspaceFile[]>('list_workspace_files', { projectId });
export const readWorkspaceFile = (projectId: string, relPath: string) =>
  ipc<string>('read_workspace_file', { projectId, relPath });
export const writeWorkspaceFile = (projectId: string, relPath: string, content: string) =>
  ipc<WorkspaceFile>('write_workspace_file', { projectId, relPath, content });

// ── Settings — LLM ──────────────────────────────────────────────────────────
export const listLlmConfigs = () => ipc<LlmConfig[]>('list_llm_configs');
export const createLlmConfig = (payload: {
  name: string; model: string;
  endpoint: string; api_key: string; temperature?: number;
  api_spec?: ApiSpec;
}) => ipc<LlmConfig>('create_llm_config', { payload });
export const updateLlmConfig = (id: string, payload: Partial<{
  name: string; model: string; endpoint: string;
  api_key: string; temperature: number; enabled: boolean;
  api_spec: ApiSpec;
}>) => ipc<LlmConfig>('update_llm_config', { id, payload });

// ── Settings — Web 搜索工具 ───────────────────────────────────────────────────
export interface WebSearchSettings {
  provider: string; endpoint: string; max_results: number; api_key_set: boolean; fetch_content: boolean;
}
export const getWebSearchSettings = () =>
  ipc<WebSearchSettings>('get_web_search_settings');
export const setWebSearchSettings = (
  provider: string, endpoint: string, max_results: number, api_key?: string, fetch_content = false,
) => ipc<WebSearchSettings>('set_web_search_settings', { provider, endpoint, maxResults: max_results, apiKey: api_key, fetchContent: fetch_content });

// ── 操作者身份卡（rail-me）────────────────────────────────────────────────────
export interface OperatorProfile {
  display_name: string; avatar: string; accent_color: string; role: string;
}
export const getOperatorProfile = () => ipc<OperatorProfile>('get_operator_profile');
export const setOperatorProfile = (profile: OperatorProfile) =>
  ipc<OperatorProfile>('set_operator_profile', { profile });

// ── 活动通知收件箱 ────────────────────────────────────────────────────────────
export type NotificationCategory = 'intervene' | 'progress' | 'result' | 'intake';
export interface AppNotification {
  id: string;
  category: NotificationCategory;
  kind: string;
  title: string;
  body: string;
  link_page: string | null;
  link_ref: string | null;
  read: boolean;
  created_at: string;
}
export interface NotificationInbox { items: AppNotification[]; unread: number; }
export const listNotifications = (limit?: number) =>
  ipc<NotificationInbox>('list_notifications', { limit });
export const unreadNotificationCount = () => ipc<number>('unread_notification_count');
export const markNotificationRead = (id: string) =>
  ipc<void>('mark_notification_read', { id });
export const markAllNotificationsRead = () => ipc<void>('mark_all_notifications_read');

// ── Settings — 语音录入（ASR）────────────────────────────────────────────────
export interface AsrSettings {
  provider: string; endpoint: string; model: string; language: string; api_key_set: boolean;
}
export const getAsrSettings = () => ipc<AsrSettings>('get_asr_settings');
export const setAsrSettings = (
  provider: string, endpoint: string, model: string, language: string, api_key?: string,
) => ipc<AsrSettings>('set_asr_settings', { provider, endpoint, model, language, apiKey: api_key });

// 会议录音整段转写：一段 16kHz/单声道/16bit PCM(base64) 经阿里百炼 DashScope 实时 WS
// 转写为完整文本（与实时麦克风同一条链路）。长录音由前端切段、逐段调用后拼接。
export const transcribeRecordingSegment = (pcm_base64: string) =>
  ipc<string>('transcribe_recording_segment', { pcmBase64: pcm_base64 });

// 兜底：浏览器 WebView 解不出某格式时，把原始文件字节(base64)交后端 symphonia 解码
// （mp3/m4a-aac/wav/flac/ogg-vorbis）→ 分段经 DashScope WS → 返回完整转写文本。
export const transcribeRecordingFile = (file_base64: string, mime?: string) =>
  ipc<string>('transcribe_recording_file', { fileBase64: file_base64, mime });

// 实时语音识别（阿里 DashScope 流式）：start 返回 session_id，结果经 AutoForge://event 的
// asr_result 事件推送；前端按 16kHz/单声道/16bit PCM 分帧 feed，结束调 stop。
export const asrRealtimeStart = () => ipc<string>('asr_realtime_start');
export const asrRealtimeFeed = (session_id: string, audio_base64: string) =>
  ipc<void>('asr_realtime_feed', { sessionId: session_id, audioBase64: audio_base64 });
export const asrRealtimeStop = (session_id: string) =>
  ipc<void>('asr_realtime_stop', { sessionId: session_id });

// ── 会议录音：转写文本 → AI 提炼纪要 + 拆解需求（先审后入）────────────────────
export interface MeetingIssue {
  title: string; category: string; severity: string; description: string;
}
export interface MeetingAnalysis {
  summary_md: string; issues: MeetingIssue[];
}
// 分析会议转写：返回纪要 + 需求列表（不落盘、不入池）。
export const analyzeMeeting = (transcript: string) =>
  ipc<MeetingAnalysis>('analyze_meeting', { transcript });
// 把会议纪要写入项目 .autoforge/docs/meetings/，返回相对路径。
export const saveMeetingDoc = (
  project_id: string, title: string, summary_md: string, transcript: string,
) => ipc<string>('save_meeting_doc', { projectId: project_id, title, summaryMd: summary_md, transcript });

// 凭据加密主密钥后端："keychain"（系统钥匙环）或 "file"（0600 文件兜底，安全性较弱）。
export type SecretBackend = 'keychain' | 'file';
export const getSecretBackendStatus = () =>
  ipc<SecretBackend>('secret_backend_status');

// ── Settings — 配置备份（口令加密导出 / 导入）────────────────────────────────
export interface BackupSummary {
  llm_configs: number; agents: number; mcp_servers: number;
  notify_channels: number; app_settings: number;
}
export interface ExportResult { path: string; summary: BackupSummary; }
/** 口令加密导出全部配置到备份目录，返回文件路径与计数。 */
export const exportConfig = (passphrase: string) =>
  ipc<ExportResult>('export_config', { passphrase });
/** 用口令解密备份文件内容并恢复配置（按主键 upsert）。 */
export const importConfig = (content: string, passphrase: string) =>
  ipc<BackupSummary>('import_config', { content, passphrase });
/** 在系统文件管理器中定位备份文件。 */
export const revealBackup = (path: string) =>
  ipc<void>('reveal_backup', { path });

// ── Settings — MCP servers ────────────────────────────────────────────────────
export type McpTransport = 'stdio' | 'http';
export interface McpServer {
  id: string; name: string; transport: McpTransport;
  command: string; args_json: string; env_json: string;
  url: string; headers_json: string; agent_ids_json: string;
  for_code_agent: boolean; capability_map_json: string;
  enabled: boolean; created_at: string;
}
export type McpServerInput = Partial<{
  name: string; transport: McpTransport;
  command: string; args_json: string; env_json: string;
  url: string; headers_json: string; agent_ids_json: string;
  for_code_agent: boolean; capability_map_json: string; enabled: boolean;
}>;
export const listMcpServers = () => ipc<McpServer[]>('list_mcp_servers');
export const createMcpServer = (payload: McpServerInput & { name: string }) =>
  ipc<McpServer>('create_mcp_server', { payload });
export const updateMcpServer = (id: string, payload: McpServerInput) =>
  ipc<McpServer>('update_mcp_server', { id, payload });
export const deleteMcpServer = (id: string) => ipc<void>('delete_mcp_server', { id });
export const testMcpConnection = (id: string) => ipc<string[]>('test_mcp_connection', { id });
export const discoverCodeIntelMap = (id: string) => ipc<string>('discover_code_intel_map', { id });
export const deleteLlmConfig = (id: string) => ipc<void>('delete_llm_config', { id });
/** 测试连接结果：状态文案 + 顺带刷新（上下文窗口 / 多模态）后的配置。 */
export interface TestLlmResult { message: string; config: LlmConfig; }
export const testLlmConnection = (id: string, draft?: Partial<LlmConfig>) =>
  ipc<TestLlmResult>('test_llm_connection', { id, draft: draft && Object.keys(draft).length ? draft : null });

// ── Settings — Agents ────────────────────────────────────────────────────────
export const listAgents = () => ipc<Agent[]>('list_agents');
export const createAgent = (payload: {
  name: string; name_en?: string; role?: string; color?: string;
  initial?: string; llm_id?: string; system_prompt?: string;
  role_type?: 'business' | 'system'; system_kind?: string | null;
  capabilities_json?: string; max_concurrency?: number;
  visible_in_chat?: boolean; mentionable?: boolean; enabled?: boolean;
  prompt_mode?: 'builtin' | 'append' | 'custom'; memory_enabled?: boolean;
}) => ipc<Agent>('create_agent', { payload });
export const updateAgent = (id: string, payload: Partial<{
  name: string; name_en: string; role: string; color: string;
  llm_id: string | null; system_prompt: string; forge_role: string | null;
  role_type: 'business' | 'system'; system_kind: string | null;
  capabilities_json: string; max_concurrency: number;
  visible_in_chat: boolean; mentionable: boolean; enabled: boolean;
  prompt_mode: 'builtin' | 'append' | 'custom'; memory_enabled: boolean;
}>) => ipc<Agent>('update_agent', { id, payload });
export const deleteAgent = (id: string) => ipc<void>('delete_agent', { id });
export const setAgentForgeRole = (agentId: string, role: string) =>
  ipc<Agent[]>('set_agent_forge_role', { agentId, role });

// ── Settings — Role catalog (两层模型) ──────────────────────────────────────
export const listRoleCatalog = () => ipc<RoleSlot[]>('list_role_catalog');
export const setRoleSlot = (kind: string, payload: {
  llm_id?: string; prompt_mode?: 'builtin' | 'append' | 'custom';
  supplement?: string; enabled?: boolean;
  visible_in_chat?: boolean; mentionable?: boolean; memory_enabled?: boolean;
  capabilities_json?: string;
}) => ipc<RoleSlot[]>('set_role_slot', { kind, payload });

// 内置工具目录：驱动 Agent 能力开关的动态渲染（新增后端工具自动出现，无需改前端）。
export interface BuiltinToolInfo { name: string; label: string; needs_project: boolean; }
export const listBuiltinTools = () => ipc<BuiltinToolInfo[]>('list_builtin_tools');

// ── LLM Trace（调用链路追踪）────────────────────────────────────────────────
export interface LlmTraceSummary {
  trace_id: string;
  agent_id: string | null; agent_role: string | null; agent_name: string | null;
  project_id: string | null; issue_id: string | null; conversation_id: string | null; task_id: string | null;
  status: string; error: string | null;
  input: string | null; output: string | null;
  total_tokens: number | null; latency_ms: number | null;
  span_count: number; created_at: string;
  innate_triggered: boolean; // root span 是否注入了 Innate 记忆召回（派生字段）
}
export interface LlmTrace {
  id: string; trace_id: string; parent_id: string | null; seq: number;
  kind: string; name: string | null;
  agent_id: string | null; agent_role: string | null; agent_name: string | null;
  project_id: string | null; issue_id: string | null; conversation_id: string | null; task_id: string | null;
  provider: string | null; model: string | null; system_prompt: string | null;
  status: string; error: string | null; input: string | null; output: string | null;
  prompt_tokens: number | null; completion_tokens: number | null; total_tokens: number | null;
  latency_ms: number | null; metadata_json: string | null;
  started_at: string | null; ended_at: string | null; created_at: string;
  innate_triggered: boolean; // 本 span 是否注入了 Innate 记忆召回（派生字段）
}
export interface TraceFilter {
  issue_id?: string; conversation_id?: string; agent_name?: string; agent_role?: string;
  agent_id?: string; project_id?: string; status?: string; kind?: string; limit?: number;
}
export const listLlmTraces = (filter: TraceFilter = {}) => ipc<LlmTraceSummary[]>('list_llm_traces', { filter });
export const getLlmTrace = (traceId: string) => ipc<LlmTrace[]>('get_llm_trace', { traceId });
export const listTraceAgentNames = () => ipc<string[]>('list_trace_agent_names');
export const clearLlmTraces = (traceId?: string) => ipc<void>('clear_llm_traces', { traceId: traceId ?? null });

// ── Agent Outputs（环节 Agent 结构化产出 / schema 驱动）─────────────────────────
export interface AgentOutputSummary {
  id: string; role: string; schema_version: string;
  target_kind: string; target_id: string;
  project_id: string | null; trace_id: string | null;
  status: string; output_json: string | null; created_at: string;
}
export interface AgentOutput extends AgentOutputSummary {
  raw: string | null;
}
export interface AgentOutputFilter {
  role?: string; target_kind?: string; target_id?: string;
  project_id?: string; status?: string; limit?: number;
}
export const listAgentOutputs = (filter: AgentOutputFilter = {}) => ipc<AgentOutputSummary[]>('list_agent_outputs', { filter });
export const getAgentOutput = (id: string) => ipc<AgentOutput | null>('get_agent_output', { id });
export const listAgentOutputRoles = () => ipc<string[]>('list_agent_output_roles');
export const clearAgentOutputs = (id?: string) => ipc<void>('clear_agent_outputs', { id: id ?? null });

export interface FieldStat { path: string; filled: number; total: number; fill_rate: number; }
export interface FieldHealth {
  role: string; schema_version: string | null; total: number;
  status_ok: number; status_partial: number; status_error: number;
  fields: FieldStat[];
}
export const agentOutputFieldHealth = (role: string, schemaVersion?: string, projectId?: string) =>
  ipc<FieldHealth>('agent_output_field_health', { role, schemaVersion: schemaVersion ?? null, projectId: projectId ?? null });

export const openUrl = (url: string) => ipc<void>('open_url', { url });

// ── Materials ─────────────────────────────────────────────────────────────────
export interface MaterialFolder {
  id: string; project_id: string; parent_id: string | null;
  name: string; sort_order: number;
  created_at: string; updated_at: string;
}
export interface MaterialFile {
  id: string; project_id: string; folder_id: string | null;
  original_name: string; stored_name: string; rel_path: string;
  mime: string; size_bytes: number; sha256: string;
  description: string | null; tags: string;
  backup_status: string; backup_url: string | null;
  backed_up_at: string | null; created_at: string; updated_at: string;
}
export interface MaterialSearchResult {
  file: MaterialFile;
  score: number;
  match_reason: string;
  content_preview: string | null;
  folder_path: string;
}
export interface MaterialBackupConfig {
  id: string; provider: string; config_json: string;
  enabled: boolean; updated_at: string;
}

export const listMaterialFolders = (projectId: string) =>
  ipc<MaterialFolder[]>('list_material_folders', { projectId });
export const createMaterialFolder = (projectId: string, parentId: string | null, name: string) =>
  ipc<MaterialFolder>('create_material_folder', { projectId, parentId, name });
export const renameMaterialFolder = (id: string, name: string) =>
  ipc<MaterialFolder>('rename_material_folder', { id, name });
export const deleteMaterialFolder = (id: string) =>
  ipc<boolean>('delete_material_folder', { id });
export const listMaterialFiles = (projectId: string, folderId?: string | null) =>
  ipc<MaterialFile[]>('list_material_files', { projectId, folderId: folderId ?? null });
export const searchMaterialFiles = (projectId: string, query: string, aiMode = false) =>
  ipc<MaterialSearchResult[]>('search_material_files', { projectId, query, aiMode });
export const importMaterialFile = (
  projectId: string, folderId: string | null,
  fileName: string, mimeHint: string, dataBase64: string,
) => ipc<MaterialFile>('import_material_file', { projectId, folderId, fileName, mimeHint, dataBase64 });
export const moveMaterialFile = (id: string, folderId: string | null) =>
  ipc<MaterialFile>('move_material_file', { id, folderId });
export const updateMaterialFileMeta = (id: string, description?: string, tags?: string) =>
  ipc<MaterialFile>('update_material_file_meta', { id, description: description ?? null, tags: tags ?? null });
export const deleteMaterialFile = (id: string) =>
  ipc<void>('delete_material_file', { id });
export const openMaterialFile = (id: string) =>
  ipc<void>('open_material_file', { id });
export const materialFileDataUrl = (id: string) =>
  ipc<string>('material_file_data_url', { id });
export const aiOrganizeMaterials = (projectId: string) =>
  ipc<string>('ai_organize_materials', { projectId });
export const getMaterialBackupConfig = () =>
  ipc<MaterialBackupConfig>('get_material_backup_config');
export const updateMaterialBackupConfig = (provider: string, configJson: string, enabled: boolean) =>
  ipc<MaterialBackupConfig>('update_material_backup_config', { provider, configJson, enabled });
export const backupMaterialFiles = (projectId: string, fileIds: string[]) =>
  ipc<string>('backup_material_files', { projectId, fileIds });

// ── Intake ───────────────────────────────────────────────────────────────────
export interface IntakeConfig {
  id: string; webhook_enabled: boolean; webhook_port: number;
  webhook_token: string; github_owner: string; github_repo: string;
  github_token: string; github_project_id: string;
  github_last_sync: string | null; ci_watch_dir: string; updated_at: string;
}
export interface SyncResult { imported: number; skipped: number; errors: number; }
export interface ScanResult { found: number; new_issues: number; }
export interface BulkResult { total: number; imported: number; skipped: number; errors: string[]; }
export interface WebhookStatus { running: boolean; port: number; }

export const getIntakeConfig = () => ipc<IntakeConfig>('get_intake_config');
export const updateIntakeConfig = (payload: Partial<{
  webhook_enabled: boolean; webhook_port: number; webhook_token: string;
  github_owner: string; github_repo: string; github_token: string;
  github_project_id: string; ci_watch_dir: string;
}>) => ipc<IntakeConfig>('update_intake_config', { payload });
export const getWebhookStatus = () => ipc<WebhookStatus>('get_webhook_status');
export const syncGithubIssues = () => ipc<SyncResult>('sync_github_issues');
export const bulkImportIssues = (projectId: string, format: string, content: string) =>
  ipc<BulkResult>('bulk_import_issues', { projectId, format, content });
// 文件批量导入（csv/xlsx/xls/ods）：文件内容以 base64 传入，后端按后缀解析。
export const bulkImportFile = (projectId: string, fileName: string, dataBase64: string) =>
  ipc<BulkResult>('bulk_import_file', { projectId, fileName, dataBase64 });
// 导出批量导入模板（csv/xlsx）：写入系统下载目录并在文件管理器中定位，返回路径。
export const exportBulkTemplate = (format: 'csv' | 'xlsx') =>
  ipc<string>('export_bulk_template', { format });
export const submitFromArtifact = (payload: {
  project_id: string; title: string; description?: string;
  category?: string; severity?: string; source_ref?: string;
}) => ipc<Issue>('submit_from_artifact', { payload });

// 在会议室对话 card 内直接确认/拒绝一条整理好的需求草稿。
// confirm → 入流水线并返回创建的 Issue；reject → 返回 null。决策持久化到该消息。
export const decideIssueDraft = (payload: {
  message_id: string; decision: 'confirm' | 'reject'; block_index?: number;
}) => ipc<Issue | null>('decide_issue_draft', { payload });

// ── Triage（待整理池）─────────────────────────────────────────────────────────
export interface RefineResult { refined: number; discarded: number; errors: number; }
export const listTriageIssues = (projectId?: string) =>
  ipc<Issue[]>('list_triage_issues', { projectId: projectId ?? null });
export const refineTriage = (issueIds: string[]) =>
  ipc<RefineResult>('refine_triage', { issueIds });
export const discardTriage = (issueId: string) =>
  ipc<void>('discard_triage', { issueId });
export interface RejectResult { deleted: number; rejected: number; skipped: number; }
export const rejectIssues = (issueIds: string[]) =>
  ipc<RejectResult>('reject_issues', { issueIds });

// ── 工厂自喂料（autosupply）────────────────────────────────────────────────────
export interface AutosupplySettings {
  enabled: boolean; interval_min: number; scan_enabled: boolean;
  proposer_enabled: boolean; max_per_run: number;
  proposer_max_per_run: number; min_severity: string;
  analyze_enabled: boolean; triage_enabled: boolean;
}
export const getAutosupplySettings = () => ipc<AutosupplySettings>('get_autosupply_settings');
export const setAutosupplySettings = (s: AutosupplySettings) =>
  ipc<AutosupplySettings>('set_autosupply_settings', {
    enabled: s.enabled, intervalMin: s.interval_min, scanEnabled: s.scan_enabled,
    proposerEnabled: s.proposer_enabled, maxPerRun: s.max_per_run,
    proposerMaxPerRun: s.proposer_max_per_run, minSeverity: s.min_severity,
    analyzeEnabled: s.analyze_enabled, triageEnabled: s.triage_enabled,
  });
export const runProposer = (projectId: string, max?: number) =>
  ipc<ScanResult>('run_proposer', { projectId, max: max ?? null });
export const runAutosupplyNow = () => ipc<ScanResult>('run_autosupply_now');
// 当前是否正有一轮自喂料在跑（状态真源在后端，供切页重挂载后恢复回显）。
export const autosupplyIsRunning = () => ipc<boolean>('autosupply_is_running');

// 信任旋钮：strict / standard / loose。应用预设到自喂料，不触碰审核闸。
export type AutonomyLevel = 'strict' | 'standard' | 'loose';
export const getAutonomyLevel = () => ipc<AutonomyLevel>('get_autonomy_level');
export const setAutonomyLevel = (level: AutonomyLevel) => ipc<AutonomyLevel>('set_autonomy_level', { level });

// ── Dev Server ───────────────────────────────────────────────────────────────
export const getDevServerStatus = (projectId: string) =>
  ipc<DevServerStatus>('get_dev_server_status', { projectId });
export const startDevServer = (projectId: string) =>
  ipc<DevServerStatus>('start_dev_server', { projectId });
export const stopDevServer = (projectId: string) =>
  ipc<void>('stop_dev_server', { projectId });
export const getDevServerLog = (projectId: string) =>
  ipc<string>('get_dev_server_log', { projectId });

// ── CR worktree preview (本次改动) ──────────────────────────────────────────────
export interface CrPreviewStatus {
  cr_id: string;
  kind: 'web' | 'tauri' | 'none';
  status: 'no_config' | 'no_session' | 'idle' | 'starting' | 'running' | 'stopped';
  url: string | null;
  can_launch_app: boolean;
  /** tauri 项目：iframe 仅渲染前端，IPC 不可用（接口无数据），真正预览是桌面窗口 */
  frontend_only: boolean;
  /** kind 由项目文件自动识别（config_yaml 未显式声明 dev.kind） */
  auto_detected: boolean;
  /** launch_cr_app 启动的桌面应用进程仍存活 —— UI 据此把「启动」切为「停止」 */
  app_running: boolean;
}
export const getCrPreview = (crId: string) =>
  ipc<CrPreviewStatus>('get_cr_preview', { crId });
export const startCrPreview = (crId: string) =>
  ipc<CrPreviewStatus>('start_cr_preview', { crId });
export const stopCrPreview = (crId: string) =>
  ipc<void>('stop_cr_preview', { crId });
export const launchCrApp = (crId: string) =>
  ipc<void>('launch_cr_app', { crId });
export const getCrPreviewLog = (crId: string) =>
  ipc<string>('get_cr_preview_log', { crId });

// ── 分支启动（左侧「启动项目」选分支，worktree 隔离，多分支并行）─────────────────
export interface BranchInfo {
  name: string;
  is_current: boolean;
  is_main: boolean;
  is_dev: boolean;
}
export interface BranchPreviewStatus {
  branch: string;
  kind: 'web' | 'tauri';
  status: 'starting' | 'running';
  url: string | null;
  can_launch_app: boolean;
}
export const listLocalBranches = (projectId: string) =>
  ipc<BranchInfo[]>('list_local_branches', { projectId });
export const startBranchPreview = (projectId: string, branch: string) =>
  ipc<BranchPreviewStatus>('start_branch_preview', { projectId, branch });
export const listBranchPreviews = (projectId: string) =>
  ipc<BranchPreviewStatus[]>('list_branch_previews', { projectId });
export const stopBranchPreview = (projectId: string, branch: string) =>
  ipc<void>('stop_branch_preview', { projectId, branch });
export const getBranchPreviewLog = (projectId: string, branch: string) =>
  ipc<string>('get_branch_preview_log', { projectId, branch });

// 预览日志实时 tail（事件驱动）：打开日志弹窗时 start、关闭时 stop。
// `sig` 即弹窗标识（`cr:<id>` 或 `branch:<pid>:<branch>`），后端据此解析日志文件并
// 通过 `preview_log` 事件（payload.key === sig）增量推送新增内容。
export const startPreviewLogTail = (sig: string) =>
  ipc<void>('start_preview_log_tail', { sig });
export const stopPreviewLogTail = (sig: string) =>
  ipc<void>('stop_preview_log_tail', { sig });

// ── Specs ─────────────────────────────────────────────────────────────────────

export type SpecCategory = 'tech_stack' | 'architecture' | 'coding' | 'api' | 'testing' | 'reference';
/** 规格来源：'db'(内容内联) | 'file'(指向 .autoforge/specs 磁盘文件) */
export type SpecSource = 'db' | 'file';
/** 注入档位：always 全文常驻 / on_demand 工具按需读 / off 不暴露 */
export type SpecInjection = 'always' | 'on_demand' | 'off';

export interface ProjectSpec {
  id: string;
  project_id: string;
  category: SpecCategory;
  title: string;
  content: string;
  sort_order: number;
  source: SpecSource;
  rel_path: string;
  description: string;
  injection: SpecInjection;
  created_at: string;
  updated_at: string;
}

export const listProjectSpecs = (projectId: string) =>
  ipc<ProjectSpec[]>('list_project_specs', { projectId });
export const upsertProjectSpec = (
  projectId: string, id: string | null, category: SpecCategory, title: string, content: string,
  description?: string, injection?: SpecInjection,
) => ipc<ProjectSpec>('upsert_project_spec', { projectId, id, category, title, content, description, injection });
export const deleteProjectSpec = (id: string) =>
  ipc<boolean>('delete_project_spec', { id });
export const aiGenerateSpecs = (projectId: string) =>
  ipc<string>('ai_generate_specs', { projectId });
/** 扫描 .autoforge/specs 目录，把自由 .md 文件对账登记为文件规格 */
export const scanSpecFiles = (projectId: string) =>
  ipc<string>('scan_spec_files', { projectId });
/** 取规格全文（db 源内联 / file 源读盘） */
export const getSpecContent = (id: string) =>
  ipc<string>('get_spec_content', { id });
/** 轻量切换注入档位 */
export const setSpecInjection = (id: string, injection: SpecInjection) =>
  ipc<boolean>('set_spec_injection', { id, injection });

// ── Delivery pipeline: prototype (03) / security (07) / deploy (08) / scan (06) ──

export interface PrototypePrompt {
  id: string; project_id: string; issue_id: string | null;
  tool_target: string; title: string; prompt: string; created_at: string;
}
export interface SecurityAudit {
  id: string; project_id: string; change_request_id: string | null;
  status: string; severity: string; summary: string;
  findings_json: string; issues_created: string;
  started_at: string; completed_at: string | null;
}
export interface Deployment {
  id: string; project_id: string; change_request_id: string | null;
  target_env: string; script: string; status: string; log: string;
  created_at: string; confirmed_at: string | null; completed_at: string | null;
}
export interface CrGrade {
  change_request_id: string; tier: string; score: number;
  rationale: string; change_class: string; created_at: string;
}
export interface AutoPassPolicy {
  change_class: string; trust_state: string;
  approve_count: number; reject_count: number; updated_at: string;
}

// Node 03 — prototype design prompts
export const listPrototypePrompts = (projectId?: string) =>
  ipc<PrototypePrompt[]>('list_prototype_prompts', { projectId: projectId ?? null });
export const generatePrototypePrompt = (projectId: string, issueId?: string | null, toolTarget?: string) =>
  ipc<PrototypePrompt>('generate_prototype_prompt', {
    projectId, issueId: issueId ?? null, toolTarget: toolTarget ?? null,
  });

// Node 07 — security audits
export const listSecurityAudits = (projectId?: string) =>
  ipc<SecurityAudit[]>('list_security_audits', { projectId: projectId ?? null });

// Node 08 — deployments
export const listDeployments = (projectId?: string) =>
  ipc<Deployment[]>('list_deployments', { projectId: projectId ?? null });
export const generateDeployScript = (projectId: string, targetEnv?: string) =>
  ipc<Deployment>('generate_deploy_script', { projectId, targetEnv: targetEnv ?? null });
export const confirmDeploy = (deploymentId: string) =>
  ipc<Deployment>('confirm_deploy', { deploymentId });
export const updateDeployScript = (deploymentId: string, script: string) =>
  ipc<Deployment>('update_deploy_script', { deploymentId, script });
export const deleteDeployment = (deploymentId: string) =>
  ipc<void>('delete_deployment', { deploymentId });

// Node 06 — proactive inspection
export const runProactiveScan = (projectId: string) =>
  ipc<void>('run_proactive_scan', { projectId });

// §7 — diff risk grade
export const getCrGrade = (crId: string) =>
  ipc<CrGrade | null>('get_cr_grade', { crId });
export const listAutoPassPolicy = () =>
  ipc<AutoPassPolicy[]>('list_auto_pass_policy');
export const getAutoPassEnabled = () =>
  ipc<boolean>('get_auto_pass_enabled');
export const setAutoPassEnabled = (enabled: boolean) =>
  ipc<void>('set_auto_pass_enabled', { enabled });
export const getAutoConflictResolveEnabled = () =>
  ipc<boolean>('get_auto_conflict_resolve_enabled');
export const setAutoConflictResolveEnabled = (enabled: boolean) =>
  ipc<void>('set_auto_conflict_resolve_enabled', { enabled });
export const getCustomMergeMessageEnabled = () =>
  ipc<boolean>('get_custom_merge_message_enabled');
export const setCustomMergeMessageEnabled = (enabled: boolean) =>
  ipc<void>('set_custom_merge_message_enabled', { enabled });
// 合并并行化开关：开启后合并拆为并行 premerge（测试出锁）+ 串行 land
export const getParallelPremergeEnabled = () =>
  ipc<boolean>('get_parallel_premerge_enabled');
export const setParallelPremergeEnabled = (enabled: boolean) =>
  ipc<void>('set_parallel_premerge_enabled', { enabled });
// 默认合并提交信息（与后端回退模板一致），审核页据此预填
export const getDefaultMergeMessage = (crId: string) =>
  ipc<string>('get_default_merge_message', { crId });

// M5 — preview data masking + container preview
export const maskPreviewData = (crId: string) =>
  ipc<void>('mask_preview_data', { crId });
export const provisionPreviewContainer = (crId: string) =>
  ipc<string>('provision_preview_container', { crId });

// M11 — notification hub
export interface NotifyChannel {
  id: string; name: string; kind: string;
  target: string; events: string; enabled: boolean;
  has_secret: boolean; created_at: string;
}
export interface NotifyChannelPayload {
  name: string; kind: string; target: string;
  events?: string; secret?: string; enabled?: boolean;
}
export const listNotifyChannels = () => ipc<NotifyChannel[]>('list_notify_channels');
export const createNotifyChannel = (payload: NotifyChannelPayload) =>
  ipc<NotifyChannel>('create_notify_channel', { payload });
export const updateNotifyChannel = (id: string, payload: NotifyChannelPayload) =>
  ipc<NotifyChannel>('update_notify_channel', { id, payload });
export const deleteNotifyChannel = (id: string) =>
  ipc<void>('delete_notify_channel', { id });
export const testNotifyChannel = (id: string) =>
  ipc<void>('test_notify_channel', { id });

// 微信 ClawBot 扫码绑定
export interface ClawbotQrStart {
  qrcode: string; qr_svg: string; qr_url: string; base_url: string;
}
export interface ClawbotQrPoll {
  status: string; base_url: string;
  bot_token: string | null; to_user_id: string | null;
  bot_id: string | null; target: string | null;
}
export const clawbotStartLogin = () => ipc<ClawbotQrStart>('clawbot_start_login');
export const clawbotPollLogin = (qrcode: string, baseUrl: string, verifyCode?: string) =>
  ipc<ClawbotQrPoll>('clawbot_poll_login', { qrcode, baseUrl, verifyCode: verifyCode ?? null });

// 项目级需求接入 token（统一承载 webhook 接入 flow / 反馈 widget triage 两类）
export interface WidgetToken {
  id: string;
  project_id: string;
  token: string;
  label: string;
  enabled: boolean;
  mode: string; // 'flow' | 'triage'
  created_at: string;
  expires_at: string | null;
  last_used_at: string | null;
}
export const getWidgetSnippet = (projectId: string) =>
  ipc<string>('get_widget_snippet', { projectId });
export const listWidgetTokens = (projectId: string) =>
  ipc<WidgetToken[]>('list_widget_tokens', { projectId });
export const createWidgetToken = (projectId: string, label?: string) =>
  ipc<WidgetToken>('create_widget_token', { projectId, label: label ?? null });
export const setWidgetTokenEnabled = (id: string, enabled: boolean) =>
  ipc<void>('set_widget_token_enabled', { id, enabled });
export const deleteWidgetToken = (id: string) =>
  ipc<void>('delete_widget_token', { id });
// 项目 webhook 接入 token（mode=flow）：需求入口展示/轮换
export const getProjectWebhookToken = (projectId: string) =>
  ipc<WidgetToken>('get_project_webhook_token', { projectId });
export const regenerateProjectWebhookToken = (projectId: string) =>
  ipc<WidgetToken>('regenerate_project_webhook_token', { projectId });

// Node 03 — prototype prompt edit / delete
export const deletePrototypePrompt = (id: string) =>
  ipc<void>('delete_prototype_prompt', { id });
export const updatePrototypePrompt = (id: string, title: string, prompt: string) =>
  ipc<PrototypePrompt>('update_prototype_prompt', { id, title, prompt });

// OpenDesign 本地服务：默认自动模式（检测/克隆 nexu-io/open-design → install → tools-dev 启动）；
// 填写「启动命令」则切到自定义模式（执行该命令并打开 URL）。repo_path 可显式指定本地检出。
export interface OpenDesignSettings { command: string; url: string; repo_path: string }
export const getOpenDesignSettings = () =>
  ipc<OpenDesignSettings>('get_opendesign_settings');
export const setOpenDesignSettings = (command: string, url: string, repoPath: string) =>
  ipc<OpenDesignSettings>('set_opendesign_settings', { command, url, repoPath });
/** 拉起本地 OpenDesign 服务，返回就绪后应打开的 URL（浏览器打开走 openUrl）。 */
export const launchOpenDesign = () => ipc<string>('launch_opendesign');
/** 取自动启动日志末尾，用于排查启动失败原因。 */
export const getOpenDesignLog = () => ipc<string>('get_opendesign_log');

// Delivery node artifacts (persisted to .autoforge/deliverables/<node>/)
export interface DeliveryArtifact {
  id: string; project_id: string; node: string; original_name: string;
  stored_name: string; rel_path: string; mime: string; size_bytes: number;
  sha256: string; description: string; created_at: string;
}
export const listDeliveryArtifacts = (projectId: string, node?: string) =>
  ipc<DeliveryArtifact[]>('list_delivery_artifacts', { projectId, node: node ?? null });
export const importDeliveryArtifact = (
  projectId: string, node: string, fileName: string, mime: string, dataBase64: string,
) => ipc<DeliveryArtifact>('import_delivery_artifact', { projectId, node, fileName, mime, dataBase64 });
export const updateDeliveryArtifactMeta = (id: string, description: string) =>
  ipc<DeliveryArtifact>('update_delivery_artifact_meta', { id, description });
export const renameDeliveryArtifact = (id: string, name: string) =>
  ipc<DeliveryArtifact>('rename_delivery_artifact', { id, name });
export const deleteDeliveryArtifact = (id: string) =>
  ipc<void>('delete_delivery_artifact', { id });
export const deliveryArtifactDataUrl = (id: string) =>
  ipc<string>('delivery_artifact_data_url', { id });
export const revealDeliveryArtifact = (id: string) =>
  ipc<void>('reveal_delivery_artifact', { id });
