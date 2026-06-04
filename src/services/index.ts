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
  system_prompt: string; forge_role: string | null; created_at: string;
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
  analysis_summary: string; raw_llm_output: string | null; created_at: string;
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
}
export interface Message {
  id: string; conversation_id: string; from_agent: string | null;
  content_json: string; created_at: string;
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
export const getProject = (id: string) => ipc<Project>('get_project', { id });
export const createProject = (payload: {
  name: string; slug: string; description?: string;
  repo_path: string; branch_dev?: string; branch_main?: string;
}) => ipc<Project>('create_project', { payload });
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
  name: string, memberIds: string[], color?: string, initial?: string,
) => ipc<Conversation>('create_group_conversation', { name, memberIds, color, initial });
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
export const agentReply = (conversationId: string, agentId: string) =>
  ipc<void>('agent_reply', { conversationId, agentId });

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
}) => ipc<Agent>('create_agent', { payload });
export const updateAgent = (id: string, payload: Partial<{
  name: string; name_en: string; role: string; color: string;
  llm_id: string | null; system_prompt: string; forge_role: string | null;
}>) => ipc<Agent>('update_agent', { id, payload });
export const deleteAgent = (id: string) => ipc<void>('delete_agent', { id });
export const setAgentForgeRole = (agentId: string, role: string) =>
  ipc<Agent[]>('set_agent_forge_role', { agentId, role });

export const openUrl = (url: string) => ipc<void>('open_url', { url });
export const seedDemoData = () => ipc<string>('seed_demo_data');
