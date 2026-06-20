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
  | { t: 'artifact'; kind: string; title: string; rows: [string, string][]; body: string;
      decided?: 'confirmed' | 'rejected';
      _meta?: { project_id?: string; title?: string; description?: string; category?: string; severity?: string } }
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
