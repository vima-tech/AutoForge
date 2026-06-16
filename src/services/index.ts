/**
 * AutoForge IPC service layer.
 * All functions call Tauri invoke() commands; if running in browser (no Tauri),
 * they throw an error that pages should handle gracefully.
 */
import { invoke } from '@tauri-apps/api/core';

// Tracks how many IPC calls are currently in-flight per command.
const _inFlight: Record<string, number> = {};

function ipc<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const t0 = performance.now();
  _inFlight[cmd] = (_inFlight[cmd] ?? 0) + 1;
  console.debug(`[IPC] → ${cmd} (in-flight: ${_inFlight[cmd]})`, args ?? '');
  return invoke<T>(cmd, args)
    .then(result => {
      const ms = (performance.now() - t0).toFixed(1);
      _inFlight[cmd]--;
      console.debug(`[IPC] ✓ ${cmd} ${ms}ms (in-flight: ${_inFlight[cmd]})`);
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
  id: string; name: string; provider: string; model: string;
  endpoint: string; api_key: string; ctx_window: string;
  temperature: number; enabled: boolean; created_at: string;
}
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
  desc: string; color: string; icon: string;
  builtin_prompt: string;
  llm_only: boolean;
  holder: Agent | null;
}
export interface Project {
  id: string; name: string; slug: string; description: string;
  repo_path: string; branch_dev: string; branch_main: string;
  status: string; config_yaml: string | null;
  created_at: string; updated_at: string;
}
export interface Issue {
  id: string; project_id: string; source_type: string;
  title: string; description: string; category: string;
  severity: string; priority: number | null; status: string;
  fingerprint: string; created_at: string; updated_at: string;
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
    duplicate_of: string | null; duplicate_hint: string; analysis_summary: string; confidence: number;
  };
  understanding: {
    problem_type: string; restated_requirement: string; user_story: string | null;
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
  try { return JSON.parse(json) as IssueAnalysisSpec; } catch { return null; }
}
export interface ChangeRequest {
  id: string; project_id: string; issue_id: string; status: string;
  admin_id: string | null; approved_at: string | null;
  admin_suggestions_1: string | null; admin_suggestions_2: string | null;
  target_branch: string; created_at: string; updated_at: string;
}
export interface WorktreeSession {
  id: string; change_request_id: string; worktree_path: string;
  branch_name: string; status: string; prompt_snapshot: string | null;
  iteration_count: number; report_content: string | null;
  started_at: string | null; completed_at: string | null;
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
}
export interface SpecDocument {
  name: string; content: string;
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

export interface DevServerStatus {
  project_id: string;
  status: 'no_config' | 'idle' | 'starting' | 'running' | 'stopped';
  url: string | null;
}

// ── System ───────────────────────────────────────────────────────────────────
export const getSystemHealth = () => ipc<SystemHealth>('system_health');
export const checkClaudeAuth = () => ipc<boolean>('check_claude_auth');
export const getPipelineStats = () => ipc<PipelineStats>('pipeline_stats');
export const getBadgeCounts = () => ipc<BadgeCounts>('get_badge_counts');
export const getConcurrencyConfig = () => ipc<ConcurrencyConfig>('get_concurrency_config');
export const updateConcurrencyConfig = (payload: Partial<{
  max_slots: number; pause_threshold: number; queue_strategy: string;
}>) => ipc<ConcurrencyConfig>('update_concurrency_config', { payload });
export const readSpec = (name: string) => ipc<SpecDocument>('read_spec', { name });
export const writeSpec = (name: string, content: string) =>
  ipc<SpecDocument>('write_spec', { name, content });
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
export const deleteProject = (id: string) => ipc<void>('delete_project', { id });

// ── Issues ───────────────────────────────────────────────────────────────────
export const listIssues = (projectId?: string) =>
  ipc<Issue[]>('list_issues', { projectId: projectId ?? null });
export const getIssue = (id: string) => ipc<Issue>('get_issue', { id });
export const getIssueAnalysis = (issueId: string) =>
  ipc<IssueAnalysis | null>('get_issue_analysis', { issueId });
export const submitIssue = (payload: {
  project_id: string; title: string; description?: string;
  category?: string; severity?: string; source_type?: string;
}) => ipc<Issue>('submit_issue', { payload });

// ── Change Requests ──────────────────────────────────────────────────────────
export const listChangeRequests = (projectId?: string, status?: string) =>
  ipc<ChangeRequest[]>('list_change_requests', {
    projectId: projectId ?? null, status: status ?? null,
  });
export const getChangeRequest = (id: string) =>
  ipc<ChangeRequest>('get_change_request', { id });
export const getWorktreeSession = (crId: string) =>
  ipc<WorktreeSession | null>('get_worktree_session', { crId });
export const getCodeDiff = (crId: string) =>
  ipc<string>('get_code_diff', { crId });
export const review1 = (issueId: string, decision: {
  decision: string; suggestions?: string; admin_id?: string;
}) => ipc<ChangeRequest>('review_1', { issueId, decision });
export const review2 = (crId: string, decision: {
  decision: string; suggestions?: string; admin_id?: string;
}) => ipc<ChangeRequest>('review_2', { crId, decision });
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
  name: string; provider: string; model: string;
  endpoint: string; api_key: string; ctx_window?: string; temperature?: number;
}) => ipc<LlmConfig>('create_llm_config', { payload });
export const updateLlmConfig = (id: string, payload: Partial<{
  name: string; provider: string; model: string; endpoint: string;
  api_key: string; ctx_window: string; temperature: number; enabled: boolean;
}>) => ipc<LlmConfig>('update_llm_config', { id, payload });
export const deleteLlmConfig = (id: string) => ipc<void>('delete_llm_config', { id });
export const testLlmConnection = (id: string) => ipc<string>('test_llm_connection', { id });

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
}) => ipc<RoleSlot[]>('set_role_slot', { kind, payload });

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
export const runCodeScan = (projectId: string, scanTypes?: string[]) =>
  ipc<ScanResult>('run_code_scan', { projectId, scanTypes: scanTypes ?? [] });
export const bulkImportIssues = (projectId: string, format: string, content: string) =>
  ipc<BulkResult>('bulk_import_issues', { projectId, format, content });
export const submitFromArtifact = (payload: {
  project_id: string; title: string; description?: string;
  category?: string; severity?: string; source_ref?: string;
}) => ipc<Issue>('submit_from_artifact', { payload });

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

// ── Specs ─────────────────────────────────────────────────────────────────────

export type SpecCategory = 'tech_stack' | 'architecture' | 'coding' | 'api' | 'testing';

export interface ProjectSpec {
  id: string;
  project_id: string;
  category: SpecCategory;
  title: string;
  content: string;
  sort_order: number;
  created_at: string;
  updated_at: string;
}

export const listProjectSpecs = (projectId: string) =>
  ipc<ProjectSpec[]>('list_project_specs', { projectId });
export const upsertProjectSpec = (
  projectId: string, id: string | null, category: SpecCategory, title: string, content: string,
) => ipc<ProjectSpec>('upsert_project_spec', { projectId, id, category, title, content });
export const deleteProjectSpec = (id: string) =>
  ipc<boolean>('delete_project_spec', { id });
export const aiGenerateSpecs = (projectId: string) =>
  ipc<string>('ai_generate_specs', { projectId });

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

// M5 — preview data masking + container preview
export const maskPreviewData = (crId: string) =>
  ipc<void>('mask_preview_data', { crId });
export const provisionPreviewContainer = (crId: string) =>
  ipc<string>('provision_preview_container', { crId });

// M11 — notification hub
export interface NotifyChannel {
  id: string; name: string; kind: string;
  target: string; events: string; enabled: boolean; created_at: string;
}
export const listNotifyChannels = () => ipc<NotifyChannel[]>('list_notify_channels');
export const createNotifyChannel = (payload: {
  name: string; kind: string; target: string; events?: string; enabled?: boolean;
}) => ipc<NotifyChannel>('create_notify_channel', { payload });
export const updateNotifyChannel = (id: string, payload: {
  name: string; kind: string; target: string; events?: string; enabled?: boolean;
}) => ipc<NotifyChannel>('update_notify_channel', { id, payload });
export const deleteNotifyChannel = (id: string) =>
  ipc<void>('delete_notify_channel', { id });
export const testNotifyChannel = (kind: string, target: string) =>
  ipc<void>('test_notify_channel', { kind, target });

// M10 — embeddable feedback widget
export const getWidgetSnippet = (projectId: string) =>
  ipc<string>('get_widget_snippet', { projectId });

// Node 03 — prototype prompt edit / delete
export const deletePrototypePrompt = (id: string) =>
  ipc<void>('delete_prototype_prompt', { id });
export const updatePrototypePrompt = (id: string, title: string, prompt: string) =>
  ipc<PrototypePrompt>('update_prototype_prompt', { id, title, prompt });

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
export const deleteDeliveryArtifact = (id: string) =>
  ipc<void>('delete_delivery_artifact', { id });
export const deliveryArtifactDataUrl = (id: string) =>
  ipc<string>('delivery_artifact_data_url', { id });
