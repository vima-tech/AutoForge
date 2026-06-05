export interface Agent {
  id: string;
  name: string;
  en: string;
  role: string;
  color: string;
  initial: string;
  llm: string;
  system: string;
  forge: string | null;
  role_type?: 'business' | 'system';
  system_kind?: string | null;
  visible_in_chat?: boolean;
  mentionable?: boolean;
  enabled?: boolean;
}

export interface Conversation {
  id: string;
  type: 'direct' | 'group';
  name?: string;
  initial?: string;
  color?: string;
  agent?: string;
  members?: string[];
  unread: number;
  time: string;
  preview: string;
}

export type BlockType =
  | { t: 'md'; md: string }
  | { t: 'code'; lang: string; code: string }
  | { t: 'typing' }
  | { t: 'file'; id?: string; name: string; meta: string; color: string; mime?: string; size?: number }
  | { t: 'image'; id?: string; label: string; meta: string; color: string; mime?: string; size?: number }
  | { t: 'quote_ref'; message_id: string; author: string; text: string; created_at: string }
  | { t: 'artifact'; kind: string; title: string; rows: [string, string][]; body: string }
  | { t: 'file_written'; path: string; name: string; preview: string; size_bytes: number; error?: boolean };

export interface Message {
  from: string;
  time: string;
  blocks: BlockType[];
  _temp?: boolean;
}

export interface StatItem {
  ic: string;
  color: string;
  val: string;
  unit: string;
  label: string;
  delta: string;
  up: boolean;
}

export interface PipelineStage {
  ic: string;
  name: string;
  cnt: number;
  state: string;
}

export interface QueueItem {
  pr: number;
  title: string;
  id: string;
  cat: string;
  sev: string;
  proj: string;
  stage: string;
}

export interface ProjectItem {
  name: string;
  color: string;
  desc: string;
  lang: string;
  backlog: number;
  status: string;
}

export interface AuditProject {
  name: string;
  color: string;
  lang: string;
  preview: string;
  live: boolean;
}

export interface Requirement {
  id: string;
  title: string;
  cat: string;
  sev: string;
  iter: number;
  files: number;
  author: string;
  time: string;
}

export interface DiffLine {
  n1: number | string;
  n2: number | string;
  t: 'add' | 'del' | 'ctx';
  code: string;
}

export interface DiffHunk {
  file: string;
  hunk: string;
  lines: DiffLine[];
}

export interface LLMConfig {
  id: string;
  name: string;
  provider: string;
  color: string;
  model: string;
  endpoint: string;
  key: string;
  ctx: string;
  temp: string;
  active: boolean;
}

export const AGENTS: Agent[] = [
  { id: 'analyst', name: '需求分析师', en: 'Analysis Agent', role: '需求真实性 · 可行性 · 优先级', color: '#8b7ad8', initial: '析', llm: 'Claude Opus 4', system: '你是 AutoForge 的需求分析 Agent。评估需求真实性、可行性、优先级，输出结构化分析报告，检测重复需求。', forge: 'analysis' },
  { id: 'coder', name: 'Claude Code', en: 'Execution Agent', role: '代码实现 · worktree · 初步测试', color: '#e8772e', initial: 'CC', llm: 'Claude Sonnet 4', system: '你是执行 Agent，在 worktree 内实现已审核的需求，遵守编码规范与权限边界，生成实现报告。', forge: null },
  { id: 'tester', name: '测试工程师', en: 'Test Agent', role: '被动响应 · 主动巡检 · 缺陷报告', color: '#4f9d6b', initial: '测', llm: 'Claude Sonnet 4', system: '你是测试 Agent，验证功能正确性、执行回归与主动巡检，生成缺陷报告并自动入队。', forge: 'test' },
  { id: 'architect', name: '架构顾问', en: 'Architect', role: '技术选型 · 模块设计 · 重构建议', color: '#4f8ed1', initial: '架', llm: 'Claude Opus 4', system: '你是架构顾问，对技术选型、模块边界和重构方案给出建议。', forge: null },
  { id: 'security', name: '安全审计', en: 'Security', role: 'Prompt 注入 · 权限边界 · 脱敏', color: '#db5a40', initial: '安', llm: 'Claude Sonnet 4', system: '你是安全审计 Agent，识别 Prompt 注入、权限越界与数据脱敏风险。', forge: null },
];

export const AGENT_MAP: Record<string, Agent> = Object.fromEntries(AGENTS.map(a => [a.id, a]));

export const CONVERSATIONS: Conversation[] = [];

export const MESSAGES: Record<string, Message[]> = {};

export const DASH = {
  stats: [] as StatItem[],
  pipeline: [] as PipelineStage[],
  queue: [] as QueueItem[],
  projects: [] as ProjectItem[],
};

export const AUDIT_PROJECTS: AuditProject[] = [];

export const REQUIREMENTS: Requirement[] = [];

export const REPORT = {
  summary: '',
  files: [] as { name: string; add: number; del: number }[],
  tests: { added: 0, passed: false, cov: '' },
  risk: '',
};

export const DIFF_HUNKS: DiffHunk[] = [];

export const LLM_CONFIGS: LLMConfig[] = [
  { id: 'l1', name: 'Claude Opus 4', provider: 'Anthropic', color: '#e8772e', model: 'claude-opus-4-20250514', endpoint: 'https://api.anthropic.com', key: 'sk-ant-•••••••••••••4a2f', ctx: '200K', temp: '0.3', active: true },
  { id: 'l2', name: 'Claude Sonnet 4', provider: 'Anthropic', color: '#8b7ad8', model: 'claude-sonnet-4-20250514', endpoint: 'https://api.anthropic.com', key: 'sk-ant-•••••••••••••9c1b', ctx: '200K', temp: '0.2', active: true },
  { id: 'l3', name: '本地消毒模型', provider: 'Ollama', color: '#4f9d6b', model: 'qwen2.5:7b-instruct', endpoint: 'http://localhost:11434', key: '—', ctx: '32K', temp: '0.0', active: true },
];
