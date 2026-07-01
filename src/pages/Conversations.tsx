import React, { useState, useRef, useEffect, useLayoutEffect, useCallback, useMemo } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import { Avatar, MeAvatar } from '../components/Avatar';
import { useOperator } from '../operator';
import { primeAgents } from '../agents-store';
import Block from '../components/Block';
import Markdown from '../components/Markdown';
import RoundAvatarStack, { type RoundNav } from '../components/RoundAvatarStack';
import { ReaderToc } from '../components/ReaderToc';
import {
  listConversations, listMessages, sendMessage, createGroupConversation,
  listAgents, updateGroupConversation, addConversationMember, removeConversationMember, deleteGroupConversation, softDeleteMessage,
  markConversationRead, importAttachment, listConversationAttachments, openAttachment,
  toggleMessageContext, startConversationTask, listConversationTasks, compressConversationContext,
  draftCodingBrief, draftCodingBriefDetailed, draftCodingBriefStream, startConversationCoding,
  archiveConversation, listConversationArchives, getConversationArchive,
  searchConversationArchives, deleteConversationArchive,
  listProjectFiles, addConversationProjectContext, removeConversationProjectContext,
  listProjects, listWorkspaceFiles, writeWorkspaceFile, ensureWorkspaceDirs,
  runConversationCommand, INNATE_SENDER, submitIssue, getAsrSettings,
  type Conversation, type Message, type Agent, type ConversationAttachment,
  type Project, type ProjectContextFile, type WorkspaceFile, type ConvCommandName,
  type ConversationArchiveSummary, type ArchiveSearchHit, type ArchivedMessage,
  type CodingBrief,
} from '../services';
import type { BlockType } from '../data/mock';
import { fmtMsgTime, fmtListTime, fmtFull } from '../utils/datetime';
import { toggleMaximizeOnDoubleClick } from '../lib/window';
import { RealtimeAsr } from '../lib/realtimeAsr';
import { registerVoiceSurface } from '../lib/voiceInput';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
const MAX_ATTACHMENT_BYTES = 10 * 1024 * 1024;
const IMAGE_EXTS = ['png', 'jpg', 'jpeg', 'webp', 'gif'];
const FILE_EXTS = [...IMAGE_EXTS, 'pdf', 'txt', 'log', 'md', 'json', 'csv', 'yaml', 'yml', 'toml'];
const IMAGE_ACCEPT = '.png,.jpg,.jpeg,.webp,.gif,image/png,image/jpeg,image/webp,image/gif';
const FILE_ACCEPT = '.png,.jpg,.jpeg,.webp,.gif,.pdf,.txt,.log,.md,.json,.csv,.yaml,.yml,.toml,image/png,image/jpeg,image/webp,image/gif,application/pdf,text/plain,text/markdown,text/csv,application/json';

interface PendingAttachment {
  id: string;
  file: File;
  mode: 'file' | 'image';
}

// 工作区文件引用：点击上下文面板的工作区文件后，作为附件引用暂存到输入框上方。
// path 是相对 .autoforge/ 的路径（如 docs/prd.md），发送时随消息携带，后端按需读取内容。
interface WorkspaceRef {
  path: string;
  name: string;
}

// ── 输入框草稿：按会话窗口隔离、跨页面切换不丢失 ──
// 每个会话各自保存一份草稿，互不共享：
//  · 内存 Map 保留完整草稿（含 File 附件与 @/# 内联标签），跨「切换页面→组件卸载→返回」
//    与会话切换都不丢失（SPA 内模块常驻，卸载组件不清空它）；
//  · 纯文本（html）另存 sessionStorage，使整页刷新后文字仍能恢复（File 无法序列化，
//    刷新后附件不保留，但文字不丢）。
interface ComposerDraft {
  html: string;
  pending: PendingAttachment[];
}
const composerDrafts = new Map<string, ComposerDraft>();
const DRAFT_SS_PREFIX = 'AutoForge:draft:';

function loadComposerDraft(convId: string): ComposerDraft {
  const mem = composerDrafts.get(convId);
  if (mem) return mem;
  try {
    const html = sessionStorage.getItem(DRAFT_SS_PREFIX + convId) ?? '';
    return { html, pending: [] };
  } catch {
    return { html: '', pending: [] };
  }
}

function saveComposerDraft(convId: string, draft: ComposerDraft) {
  if (!draft.html.trim() && draft.pending.length === 0) {
    composerDrafts.delete(convId);
  } else {
    composerDrafts.set(convId, draft);
  }
  try {
    if (draft.html.trim()) sessionStorage.setItem(DRAFT_SS_PREFIX + convId, draft.html);
    else sessionStorage.removeItem(DRAFT_SS_PREFIX + convId);
  } catch {
    /* ignore */
  }
}

function clearComposerDraft(convId: string) {
  composerDrafts.delete(convId);
  try { sessionStorage.removeItem(DRAFT_SS_PREFIX + convId); } catch { /* ignore */ }
}

// 群聊 @快捷指令：`@所有人` 展开为全部可点名成员，让大家一起讨论并尽快达成一致。
const ALL_MENTION_ID = '__all__';
type MentionItem = { kind: 'all' } | { kind: 'agent'; agent: Agent };

// 内置「编码 Agent」虚拟成员：与后端 orchestration.rs 的 BUILTIN_CODE_AGENT_ID 同值。
// 它不入库，仅在「绑定了项目的群聊」里作为可 @ 成员出现，被 @ 时由后端用当前系统默认
// 编码 Agent（设置→编码 Agent 里选的默认）只读跑仓库作答。改这个值需两端同步。
const BUILTIN_CODE_AGENT_ID = '__builtin_code_agent__';
// 合成其前端 Agent 对象，供 @ 候选列表与消息作者/头像查表渲染。mentionable/visible_in_chat
// 置 false 使其**不混入**成员选择器/快速添加等通用 `agents.filter(...)` 列表（它不是可手动
// 增删的成员）；@ 候选由 members memo 显式追加，不受这两个开关影响。
function builtinCodeAgent(): Agent {
  return {
    id: BUILTIN_CODE_AGENT_ID,
    name: '编码 Agent', name_en: 'CodeAgent',
    role: '只读读项目真实代码答疑',
    color: 'var(--ember)', initial: '码', llm_id: null,
    system_prompt: '', forge_role: null,
    role_type: 'system', system_kind: null,
    capabilities_json: '[]', max_concurrency: 1,
    visible_in_chat: false, mentionable: false, enabled: true,
    prompt_mode: 'builtin', memory_enabled: false,
    code_agent_id: BUILTIN_CODE_AGENT_ID, created_at: '',
  };
}

// Innate 知识库斜杠命令：在私聊/群聊里手动触发记忆存储、召回、进化与状态查看。
interface SlashCommand { name: ConvCommandName; usage: string; desc: string; icon: string; }
const SLASH_COMMANDS: SlashCommand[] = [
  { name: 'remember', usage: '/remember [内容]', desc: '存入知识库（留空则记住最近对话）', icon: 'brain' },
  { name: 'recall',   usage: '/recall 关键词',   desc: '召回相关经验',                   icon: 'search' },
  { name: 'evolve',   usage: '/evolve',          desc: '立即蒸馏整理（进化）',           icon: 'zap' },
  { name: 'innate',   usage: '/innate',          desc: '查看知识库健康度',               icon: 'flask' },
];
// 群聊快捷指令 tag：放在 composer-tools 行，一键触发常用指令，省去重复打字。
// 带 compress 的指令（总结内容/形成结论）会在生成摘要的同时压缩上下文：
// 把当前窗口内的历史消息移出后续 Agent 上下文，让摘要成为新的上下文基线。
interface QuickPrompt {
  label: string;
  icon: string;
  /** 普通指令：作为用户消息发送并触发编排。 */
  prompt?: string;
  /** 压缩指令：summary=压缩摘要，conclusion=收敛结论；二者都会压缩上下文。 */
  compress?: 'summary' | 'conclusion';
}
const QUICK_PROMPTS: QuickPrompt[] = [
  { label: '总结内容', icon: 'log', compress: 'summary' },
  { label: '形成结论', icon: 'check', compress: 'conclusion' },
  { label: '列出待办', icon: 'quote', prompt: '请从以上讨论中提取所有待办事项，标注负责人（若有）与优先级。' },
];

/** 解析以 `/` 开头的输入；返回命令与参数，未知命令 name 为 null。 */
function parseSlashCommand(text: string): { name: ConvCommandName | null; raw: string; arg: string } | null {
  const t = text.trimStart();
  if (!t.startsWith('/')) return null;
  const m = t.slice(1).match(/^(\S+)\s*([\s\S]*)$/);
  const raw = m ? m[1].toLowerCase() : '';
  const arg = m ? m[2].trim() : '';
  const known = SLASH_COMMANDS.find(c => c.name === raw);
  return { name: known ? known.name : null, raw, arg };
}

interface QuoteDraft {
  message_id: string;
  author: string;
  text: string;
  created_at: string;
}

interface BubbleMenuState {
  x: number;
  y: number;
  message: Message;
  author: string;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

function msgText(m: Message): string {
  try {
    const blocks: BlockType[] = JSON.parse(m.content_json);
    return blocks.map(b => {
      if (b.t === 'md') return b.md;
      if (b.t === 'code') return b.code;
      if (b.t === 'file') return `${b.name} ${b.meta}`;
      if (b.t === 'image') return `${b.label} ${b.meta}`;
      if (b.t === 'quote_ref') return '';
      if (b.t === 'artifact') return `${b.kind} ${b.title} ${b.body}`;
      return '';
    }).join('\n');
  } catch {
    return m.content_json;
  }
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(bytes < 10 * 1024 ? 1 : 0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function fileExt(name: string): string {
  const idx = name.lastIndexOf('.');
  return idx >= 0 ? name.slice(idx + 1).toLowerCase() : '';
}

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error('无法读取附件'));
    reader.onload = () => {
      const result = String(reader.result ?? '');
      const comma = result.indexOf(',');
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.readAsDataURL(file);
  });
}

function parseMessageBlocks(m: Message): BlockType[] {
  try {
    return JSON.parse(m.content_json);
  } catch {
    return [{ t: 'md', md: m.content_json }];
  }
}

function messageQuote(m: Message): QuoteDraft | null {
  const quote = parseMessageBlocks(m).find(b => b.t === 'quote_ref');
  if (!quote || quote.t !== 'quote_ref') return null;
  return {
    message_id: quote.message_id,
    author: quote.author,
    text: quote.text,
    created_at: quote.created_at,
  };
}

function visibleMessageBlocks(m: Message): BlockType[] {
  return parseMessageBlocks(m).filter(b => b.t !== 'quote_ref');
}

// 抽取文档流消息的 h1–h3 标题，生成右侧大纲（TOC）。跳过围栏代码块内的 # 行，
// 仅收 1–3 级——与 DOM 中 querySelectorAll('h1,h2,h3') 的集合/顺序一致，索引可直接对应。
function docHeadings(blocks: BlockType[]): { level: number; text: string }[] {
  const md = blocks
    .filter((b): b is Extract<BlockType, { t: 'md' }> => b.t === 'md')
    .map(b => b.md)
    .join('\n');
  if (!md) return [];
  const out: { level: number; text: string }[] = [];
  let fenced = false;
  for (const ln of md.split('\n')) {
    if (/^\s*(```+|~~~+)/.test(ln)) { fenced = !fenced; continue; }
    if (fenced) continue;
    const h = ln.match(/^(#{1,3})\s+(.+)$/);
    if (h) out.push({ level: h[1].length, text: h[2].replace(/[*`]/g, '').trim() });
  }
  return out;
}

// 长文档右侧 sticky 大纲。点击平滑滚动到气泡内第 i 个标题。
function DocToc({ headings, onJump }: { headings: { level: number; text: string }[]; onJump: (i: number) => void }) {
  return (
    <nav className="doc-toc" aria-label="文档大纲">
      <div className="doc-toc-label">大纲</div>
      {headings.map((h, i) => (
        <a key={i} className={`lvl-${h.level}`} title={h.text} onClick={() => onJump(i)}>{h.text}</a>
      ))}
    </nav>
  );
}

async function copyText(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const ta = document.createElement('textarea');
  ta.value = text;
  ta.style.position = 'fixed';
  ta.style.left = '-9999px';
  document.body.appendChild(ta);
  ta.select();
  document.execCommand('copy');
  document.body.removeChild(ta);
}

function attachmentBlock(a: ConversationAttachment): BlockType {
  const meta = `${a.mime.split('/').pop()?.toUpperCase() ?? 'FILE'} · ${formatBytes(a.size_bytes)}`;
  if (a.kind === 'image') {
    return { t: 'image', id: a.id, label: a.original_name, meta, color: '#4f8ed1', mime: a.mime, size: a.size_bytes };
  }
  return { t: 'file', id: a.id, name: a.original_name, meta, color: '#e0a32e', mime: a.mime, size: a.size_bytes };
}

function contextAttachmentBlock(a: ConversationAttachment): BlockType {
  const meta = `上下文 · ${a.mime.split('/').pop()?.toUpperCase() ?? 'FILE'} · ${formatBytes(a.size_bytes)}`;
  if (a.kind === 'image') {
    return { t: 'image', id: a.id, label: a.original_name, meta, color: '#4f8ed1', mime: a.mime, size: a.size_bytes };
  }
  return { t: 'file', id: a.id, name: a.original_name, meta, color: '#e0a32e', mime: a.mime, size: a.size_bytes };
}

// 工作区文件引用块：消息携带 .autoforge/ 相对路径，后端在构建 Agent 提示时按需读取内容。
function workspaceRefBlock(r: WorkspaceRef): BlockType {
  return { t: 'ws_ref', path: r.path, name: r.name };
}

function convPreview(c: Conversation): string {
  if (!c.last_message) return '暂无消息';
  try {
    const bs: BlockType[] = JSON.parse(c.last_message);
    if (bs[0]?.t === 'md') return bs[0].md.slice(0, 40);
    if (bs[0]?.t === 'file') return `附件：${bs[0].name}`;
    if (bs[0]?.t === 'image') return `图片：${bs[0].label}`;
    return '消息';
  } catch {
    return c.last_message.slice(0, 40);
  }
}

// ─── Sub-components ───────────────────────────────────────────────────────────

function ConvItem({ c, active, agentMap, onSelect }: {
  c: Conversation; active: string;
  agentMap: Record<string, Agent>; onSelect: (id: string) => void;
}) {
  const isG = c.conv_type === 'group';
  const a = isG ? null : agentMap[c.members[0]];
  const t = isG ? (c.name ?? '群聊') : (a?.name ?? 'Agent');
  return (
    <div className={'conv-item' + (active === c.id ? ' active' : '')} onClick={() => onSelect(c.id)}>
      {isG
        ? <div className="av sq" style={{ width: 46, height: 46, background: c.color, fontSize: 'var(--text-heading)' }}>{c.initial ?? c.name?.[0] ?? '群'}</div>
        : a ? <Avatar agent={a} size={46} status={c.unread > 0 ? 'online' : undefined} />
            : <div className="av" style={{ width: 46, height: 46, background: '#888' }}>?</div>}
      <div className="conv-main">
        <div className="conv-top">
          <span className="conv-name">{t}</span>
          <span className="conv-time">
            {c.last_time ? fmtListTime(c.last_time) : ''}
          </span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <span className="conv-preview">
            {isG && <Icon name="bot" size={11} style={{ verticalAlign: -1, marginRight: 3, color: 'var(--text-faint)' }} />}
            {convPreview(c)}
          </span>
          {isG && c.project_id && (
            <span style={{ marginLeft: c.unread > 0 ? 0 : 'auto', fontSize: 'var(--text-micro)', color: 'var(--ember)', display: 'inline-flex', alignItems: 'center', gap: 2, flexShrink: 0 }}>
              <Icon name="folder" size={9} />
            </span>
          )}
          {isG && c.unread > 0 && <span className="conv-unread" style={{ marginLeft: 'auto' }}>{c.unread}</span>}
        </div>
      </div>
    </div>
  );
}

function ConvList({ convs, agents, active, onSelect, onNew, onOpenArchive, collapsed, onToggleCollapse }: {
  convs: Conversation[]; agents: Agent[];
  active: string; onSelect: (id: string) => void; onNew: () => void; onOpenArchive: () => void;
  collapsed: boolean; onToggleCollapse: () => void;
}) {
  const [q, setQ] = useState('');
  const chatAgents = useMemo(() => agents.filter(a => a.visible_in_chat && a.enabled), [agents]);
  const agentMap = useMemo(() => Object.fromEntries(chatAgents.map(a => [a.id, a])), [chatAgents]);
  const title = (c: Conversation) => c.conv_type === 'group' ? (c.name ?? '群聊') : (agentMap[c.members[0]]?.name ?? 'Agent');
  const match = (c: Conversation) => !q || title(c).toLowerCase().includes(q.toLowerCase());
  const groups  = useMemo(() => convs.filter(c => c.conv_type === 'group'),  [convs]);
  const directs = useMemo(
    () => convs.filter(c => c.conv_type === 'direct' && c.members.some(id => !!agentMap[id])),
    [convs, agentMap],
  );
  if (collapsed) return null;
  return (
    <div className="list-col">
      <div className="list-head">
        <div className="list-title-row">
          <span className="list-title">会议室</span>
          <div style={{ display: 'flex', alignItems: 'center', gap: 2 }}>
            <button className="icon-btn" title="收起对话列表" onClick={onToggleCollapse}>
              <Icon name="columns" size={18} />
            </button>
            <button className="icon-btn" title="归档区 · 检索回顾" onClick={onOpenArchive}>
              <Icon name="inbox" size={18} />
            </button>
            <button className="icon-btn" title="新建群聊" onClick={onNew} style={{ color: 'var(--ember)' }}>
              <Icon name="plus" size={20} />
            </button>
          </div>
        </div>
        <div className="search">
          <Icon name="search" size={15} />
          <input placeholder="搜索 Agent 或群聊" value={q} onChange={e => setQ(e.target.value)} />
        </div>
      </div>
      <div className="list-body scroll">
        <div className="list-group-label">群聊 · 需求讨论</div>
        {groups.filter(match).map(c => <ConvItem key={c.id} c={c} active={active} agentMap={agentMap} onSelect={onSelect} />)}
        <div className="list-group-label">Agent · 单独会议室</div>
        {directs.filter(match).map(c => <ConvItem key={c.id} c={c} active={active} agentMap={agentMap} onSelect={onSelect} />)}
      </div>
    </div>
  );
}

function MessageRow({ m, agents, isGroup, highlighted, searchTerm, rowRef, onBubbleContextMenu, projectId, receipt }: {
  m: Message; agents: Agent[]; isGroup: boolean;
  highlighted?: boolean; searchTerm?: string; rowRef?: (el: HTMLDivElement | null) => void;
  onBubbleContextMenu?: (e: React.MouseEvent, message: Message, author: string) => void;
  projectId?: string;
  receipt?: boolean;
}) {
  const agentMap = useMemo(() => Object.fromEntries(agents.map(a => [a.id, a])), [agents]);
  const op = useOperator();
  const isInnate = m.from_agent === INNATE_SENDER;
  const me = !m.from_agent;
  const a  = me || isInnate ? null : agentMap[m.from_agent!];
  const author = me ? op.display_name : isInnate ? 'Innate' : (a?.name ?? 'Agent');
  const blocks = visibleMessageBlocks(m);
  const quote = messageQuote(m);
  // Agent/Innate 回复一律用「文档流」（bubble doc）统一呈现；「我」的消息保持气泡右对齐。
  const longDoc = !me;
  const bubbleRef = useRef<HTMLDivElement>(null);
  const headings = useMemo(() => (longDoc ? docHeadings(blocks) : []), [longDoc, blocks]);
  const showToc = headings.length >= 3;
  const jumpToHeading = (i: number) => {
    bubbleRef.current?.querySelectorAll('h1,h2,h3')[i]?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };
  return (
    <div ref={rowRef} className={'msg' + (me ? ' me' : '') + (longDoc ? ' msg-doc' : '') + (highlighted ? ' search-hit' : '') + ' rise'}>
      {me
        ? <MeAvatar size={36} />
        : isInnate
            ? <div className="av" style={{ width: 36, height: 36, background: 'var(--ember-tint-strong)', color: 'var(--ember-soft)', display: 'flex', alignItems: 'center', justifyContent: 'center' }} title="Innate 知识库"><Icon name="brain" size={20} /></div>
        : isGroup
            // 群聊里 agent 头像不在每条消息上重复显示——统一由左上角堆叠按发言序展示。
            // 保留 36px 占位维持消息缩进，让堆叠浮在这条头像列里。
            ? <div className="av" style={{ width: 36, height: 36, visibility: 'hidden' }} />
        : a ? <Avatar agent={a} size={36} />
            : <div className="av" style={{ width: 36, height: 36, background: '#888', fontSize: 'var(--text-body)' }}>?</div>}
      <div className="msg-body">
        {!me && (a || isInnate) && (
          <div className="msg-meta">
            <span className="msg-author" style={{ color: isInnate ? 'var(--ember)' : a!.color }}>{author}</span>
            {isInnate
              ? <span className="chip ember" style={{ padding: '0px 6px', fontSize: 'var(--text-micro)' }}>KNOWLEDGE</span>
              : isGroup && <span className="chip" style={{ padding: '0px 6px', fontSize: 'var(--text-micro)' }}>{a!.name_en}</span>}
            <span className="msg-time" title={fmtFull(m.created_at)}>
              {fmtMsgTime(m.created_at)}
            </span>
          </div>
        )}
        <div
          ref={bubbleRef}
          className={'bubble' + (longDoc ? ' doc' : '')}
          onContextMenu={e => onBubbleContextMenu?.(e, m, author)}
          style={m.excluded_from_context ? { opacity: 0.45, outline: '1.5px dashed var(--border-strong)', outlineOffset: 2 } : undefined}
        >
          {blocks.map((b, i) => <Block key={i} b={b} projectId={projectId} highlight={searchTerm} messageId={m.id} blockIndex={i} />)}
          {m.excluded_from_context && (
            <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 4, display: 'flex', alignItems: 'center', gap: 4 }}>
              <Icon name="eye-off" size={11} />已从 AI 上下文排除
            </div>
          )}
        </div>
        {quote && (
          <div className="bubble-quote" title={`${quote.author}: ${quote.text}`}>
            <span className="quote-author">{quote.author}</span>
            <span className="quote-text">{quote.text}</span>
          </div>
        )}
        {me && receipt && (
          <div className="msg-receipt"><Icon name="check" size={11} />已送达</div>
        )}
      </div>
      {showToc && <DocToc headings={headings} onJump={jumpToHeading} />}
    </div>
  );
}

// 把结构化的 CodingBrief 组装成人性化、可读、可编辑的 Markdown 工单。
// 这是提交给编码 Agent 的需求正文，也是「预览」标签渲染的内容。
function formatBriefMarkdown(b: CodingBrief): string {
  const parts: string[] = [];
  if (b.functional_points.length > 0) {
    parts.push('## 功能点\n' + b.functional_points.map(p => `- ${p}`).join('\n'));
  }
  if (b.involved_modules.length > 0) {
    parts.push('## 涉及模块\n' + b.involved_modules.map(m => `- \`${m}\``).join('\n'));
  }
  if (b.constraints.length > 0) {
    parts.push('## 关键约束\n' + b.constraints.map(c => `- ${c}`).join('\n'));
  }
  if (b.acceptance_criteria.length > 0) {
    parts.push('## 验收要点\n' + b.acceptance_criteria.map(a => `- ${a}`).join('\n'));
  }
  // 解析失败兜底：直接给原文，至少不丢内容。
  if (parts.length === 0) return b.raw_text.trim();
  return parts.join('\n\n');
}

// 会议室「立即编码」确认弹窗：自动梳理讨论 → 展示思考过程 → 操作者可编辑确认 → 创建需求+CR。
// 跳过需求审核闸（操作者点「立即编码」即需求侧决策），代码审核仍是合并前唯一闸门。
// 遵守 DESIGN：inset:var(--win-gutter)、不点遮罩关闭、仅 ✕/Esc、每屏 ≤1 个 btn-primary。
function CodeNowModal({ conversationId, onClose, onError }: {
  conversationId: string;
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const [title, setTitle] = useState('');
  const [brief, setBrief] = useState('');
  const [drafting, setDrafting] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const [briefData, setBriefData] = useState<CodingBrief | null>(null);
  const [thinkLog, setThinkLog] = useState('');
  const [elapsed, setElapsed] = useState(0);
  const [viewMode, setViewMode] = useState<'preview' | 'edit'>('preview');
  const [showThink, setShowThink] = useState(false);
  const thinkLogRef = useRef<HTMLDivElement>(null);
  // 组件卸载标记：避免流式 promise 在弹窗已关闭后还 setState（取消后仍「运行中」的根因之一）。
  const aliveRef = useRef(true);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  // 存活标记：挂载置 true、卸载置 false。
  // ⚠️ 必须在 mount 时重置为 true，不能只在 cleanup 里置 false：React 18 StrictMode（dev）会
  // mount→卸载(置 false)→**重挂载**，而 useRef 跨这次模拟重挂载是同一对象。若不在 body 里重置，
  // 重挂载后 aliveRef 永远停在 false，导致 handleDraft 的 promise resolve 后被 `!aliveRef` 拦截，
  // 既不填表也不 setDrafting(false) → 文本已全出、计时器却永涨、弹窗毫无变化（即本 bug 症状）。
  useEffect(() => {
    aliveRef.current = true;
    return () => { aliveRef.current = false; };
  }, []);

  // 思考日志增长时自动滚到底部
  useEffect(() => {
    if (thinkLogRef.current) {
      thinkLogRef.current.scrollTop = thinkLogRef.current.scrollHeight;
    }
  }, [thinkLog]);

  // 等待计时：每秒 +1，让「连接 AI」阶段始终有可见进展，不再像冻结
  useEffect(() => {
    if (!drafting) return;
    const t = setInterval(() => setElapsed(e => e + 1), 1000);
    return () => clearInterval(t);
  }, [drafting]);

  // 弹窗打开：先订阅流式增量事件（避免竞态丢失开头），再启动梳理。
  // ⚠️ 必须用 .then 而非 async/await：React 18 StrictMode（开发模式）会 mount→unmount→mount，
  // 若在 `await listen` 解析前 cleanup 已跑，旧写法因 unlisten 仍为 null 而漏取消订阅，导致
  // 两个监听器同时存活、每个 chunk 被累加两次（界面表现为「每个字都重复」）。此处一旦 cleanup
  // 已执行（disposed）就立刻取消刚解析出的订阅，保证任何时刻只有一个监听器、handleDraft 只触发一次。
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    listen<{ type: string; conversation_id?: string; chunk?: string }>(
      'autoforge://event',
      (e) => {
        const p = e.payload;
        if (p?.type === 'coding_brief_chunk' && p.conversation_id === conversationId && p.chunk) {
          setThinkLog(prev => prev + p.chunk);
        }
      },
    ).then(fn => {
      if (disposed) { fn(); return; }  // cleanup 已先跑：立即退订，杜绝泄漏/重复
      unlisten = fn;
      handleDraft();                    // 订阅就绪后再启动，避免丢失开头增量
    });
    return () => { disposed = true; if (unlisten) unlisten(); };
  }, [conversationId]);

  // AI 梳理：流式调用，实时显示思考过程，完成后组装人性化工单并填表
  const handleDraft = async () => {
    setDrafting(true);
    setThinkLog('');
    setElapsed(0);
    try {
      const codingBrief = await draftCodingBriefStream(conversationId);
      if (!aliveRef.current) return;
      setBriefData(codingBrief);
      setTitle(codingBrief.title);
      setBrief(formatBriefMarkdown(codingBrief));
      setViewMode('preview');
    } catch (e) {
      if (aliveRef.current) onError(`梳理功能点失败：${String(e)}`);
    } finally {
      if (aliveRef.current) setDrafting(false);
    }
  };

  const handleStart = async () => {
    if (!title.trim() || !brief.trim()) { onError('请填写需求标题与功能点'); return; }
    setSubmitting(true);
    try {
      const cr = await startConversationCoding({
        conversation_id: conversationId,
        title: title.trim(),
        brief: brief.trim(),
        client_request_id: crypto.randomUUID(),
      });
      setDone(cr.id);
      setTimeout(onClose, 1600);
    } catch (e) {
      onError(`创建编码任务失败：${String(e)}`);
      setSubmitting(false);
    }
  };

  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 240 }}>
      <div style={{ width: 540, maxWidth: '92vw', background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 12 }}>
          <Icon name="zap" size={20} style={{ color: 'var(--ember)' }} />
          <div>
            <div className="eyebrow" style={{ fontSize: 'var(--text-section)' }}><span className="cn">立即编码</span></div>
            <div style={{ fontSize: 'var(--text-control)', color: 'var(--text-3)', marginTop: 4 }}>据本次讨论创建需求并直接交编码 Agent 实现（跳过需求审核，仍走代码审核）</div>
          </div>
          <button className="icon-btn" style={{ marginLeft: 'auto' }} onClick={onClose}><Icon name="x" size={18} /></button>
        </div>

        {done ? (
          <div style={{ padding: '28px 20px', textAlign: 'center' }}>
            <Icon name="check" size={28} style={{ color: 'var(--green)' }} />
            <div style={{ marginTop: 10, fontSize: 'var(--text-body)', color: 'var(--text)' }}>已创建需求并开始编码</div>
            <div style={{ marginTop: 6, fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>CR {done}</div>
            <div style={{ marginTop: 6, fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>进度与代码审核请见「变更审核」页</div>
          </div>
        ) : (
          <>
            {/* 梳理中：实时流式思考日志 */}
            {drafting ? (
              <div style={{ padding: '18px 20px' }}>
                {/* 标题行 + 计时，让等待始终有可见进展 */}
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 14 }}>
                  <Icon name="brain" size={22} style={{ color: 'var(--ember)', animation: 'pulse 2s ease-in-out infinite' }} />
                  <div style={{ fontSize: 'var(--text-body)', color: 'var(--text)', fontWeight: 500 }}>
                    {thinkLog ? 'AI 正在梳理讨论' : '正在连接 AI 并收集上下文'}
                  </div>
                  <span style={{ marginLeft: 'auto', fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>
                    {elapsed}s
                  </span>
                  <Icon name="refresh" size={14} className="spin" style={{ color: 'var(--text-3)' }} />
                </div>

                {/* 流式思考日志 */}
                <div
                  ref={thinkLogRef}
                  style={{
                    height: 280,
                    overflowY: 'auto',
                    padding: 14,
                    background: 'var(--code-bg)',
                    borderRadius: 'var(--radius)',
                    border: '1px solid var(--border)',
                    fontFamily: 'var(--font-mono)',
                    fontSize: 'var(--text-caption)',
                    lineHeight: 'var(--leading-relaxed)',
                    color: 'var(--text-2)',
                    whiteSpace: 'pre-wrap',
                    wordBreak: 'break-word',
                  }}
                >
                  {thinkLog || (
                    <span style={{ color: 'var(--text-faint)' }}>
                      正在收集会议室讨论与项目上下文，随后 AI 开始逐字生成…
                    </span>
                  )}
                  {thinkLog && (
                    <span style={{
                      display: 'inline-block',
                      width: 7,
                      height: 14,
                      marginLeft: 2,
                      background: 'var(--ember)',
                      verticalAlign: 'text-bottom',
                      animation: 'pulse 1s steps(2) infinite',
                    }} />
                  )}
                </div>

                <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', marginTop: 10, display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Icon name="layers" size={13} />
                  <span>分析需求 · 识别关键模块 · 评估风险等级</span>
                </div>
              </div>
            ) : (
              <div style={{ padding: '16px 20px' }}>
                {/* 梳理结果摘要 */}
                {briefData && (
                  <div style={{ marginBottom: 14, padding: 12, background: 'var(--bg-3)', borderRadius: 'var(--radius)', borderLeft: `3px solid var(--ember)` }}>
                    <div style={{ display: 'flex', gap: 6, marginBottom: 8, flexWrap: 'wrap' }}>
                      {briefData.requirement_type && (
                        <span className="chip" style={{ fontSize: 'var(--text-caption)' }}>{briefData.requirement_type}</span>
                      )}
                      {briefData.risk_level && (
                        <span className="chip" style={{
                          fontSize: 'var(--text-caption)',
                          background: briefData.risk_level === '高' ? 'var(--red-tint)' : briefData.risk_level === '中' ? 'var(--amber-tint)' : 'var(--green-tint)',
                          color: briefData.risk_level === '高' ? 'var(--red-soft)' : briefData.risk_level === '中' ? 'var(--amber-soft)' : 'var(--green-soft)',
                        }}>
                          风险：{briefData.risk_level}
                        </span>
                      )}
                      {briefData.involved_modules.length > 0 && (
                        <span className="chip" style={{ fontSize: 'var(--text-caption)' }}>📁 {briefData.involved_modules.length} 模块</span>
                      )}
                      {briefData.constraints.length > 0 && (
                        <span className="chip" style={{ fontSize: 'var(--text-caption)' }}>⚡ {briefData.constraints.length} 约束</span>
                      )}
                    </div>
                  </div>
                )}

                <div className="field" style={{ marginBottom: 14 }}>
                  <label>需求标题</label>
                  <input value={title} onChange={e => setTitle(e.target.value)} placeholder="一句话描述要实现的需求" disabled={submitting} />
                </div>

                {/* 需求工单：预览（渲染 Markdown，人性化）/ 编辑（纯文本，可改） */}
                <div className="field" style={{ marginBottom: 6 }}>
                  <label style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <span>需求工单</span>
                    <div className="seg" style={{ marginLeft: 'auto' }}>
                      <button
                        type="button"
                        className={viewMode === 'preview' ? 'on' : ''}
                        onClick={() => setViewMode('preview')}
                        disabled={submitting}
                      >预览</button>
                      <button
                        type="button"
                        className={viewMode === 'edit' ? 'on' : ''}
                        onClick={() => setViewMode('edit')}
                        disabled={submitting}
                      >编辑</button>
                    </div>
                  </label>
                  {viewMode === 'preview' ? (
                    <div
                      onClick={() => !submitting && setViewMode('edit')}
                      title="点击编辑"
                      style={{
                        minHeight: 180,
                        maxHeight: 320,
                        overflowY: 'auto',
                        padding: '12px 14px',
                        background: 'var(--bg-3)',
                        border: '1px solid var(--border-strong)',
                        borderRadius: 9,
                        cursor: submitting ? 'default' : 'text',
                      }}
                    >
                      {brief.trim()
                        ? <Markdown md={brief} />
                        : <span style={{ color: 'var(--text-faint)', fontSize: 'var(--text-control)' }}>暂无内容，点此切换到编辑填写</span>}
                    </div>
                  ) : (
                    <textarea
                      value={brief}
                      onChange={e => setBrief(e.target.value)}
                      placeholder="要实现的功能点、涉及模块/文件范围、关键约束、验收要点。支持 Markdown，可自由编辑后切回预览确认。"
                      disabled={submitting}
                      autoFocus
                      style={{ minHeight: 180, resize: 'vertical', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-control)', lineHeight: 'var(--leading-normal)' }}
                    />
                  )}
                </div>

                {/* 可折叠：回看 AI 的原始思考过程（即便上游缓冲、token 一次性涌来也能复查） */}
                {thinkLog.trim() && (
                  <div style={{ marginTop: 10 }}>
                    <button
                      type="button"
                      onClick={() => setShowThink(v => !v)}
                      style={{ display: 'flex', alignItems: 'center', gap: 6, background: 'none', border: 'none', padding: 0, cursor: 'pointer', color: 'var(--text-3)', fontSize: 'var(--text-caption)' }}
                    >
                      <Icon name={showThink ? 'chevron-down' : 'chevron-right'} size={13} />
                      <span>AI 思考过程</span>
                    </button>
                    {showThink && (
                      <div style={{
                        marginTop: 8,
                        maxHeight: 200,
                        overflowY: 'auto',
                        padding: 12,
                        background: 'var(--code-bg)',
                        borderRadius: 'var(--radius)',
                        border: '1px solid var(--border)',
                        fontFamily: 'var(--font-mono)',
                        fontSize: 'var(--text-caption)',
                        lineHeight: 'var(--leading-relaxed)',
                        color: 'var(--text-3)',
                        whiteSpace: 'pre-wrap',
                        wordBreak: 'break-word',
                      }}>
                        {thinkLog}
                      </div>
                    )}
                  </div>
                )}

                <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', display: 'flex', alignItems: 'center', gap: 6, marginTop: 10 }}>
                  <Icon name="layers" size={13} />
                  <span>将自动附带最近会话快照与项目上下文文档作为编码背景</span>
                </div>
              </div>
            )}

            <div style={{ padding: '14px 20px 18px', borderTop: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 10 }}>
              {/* 重新梳理：结果不满意时一键重跑，无需关弹窗重开（梳理中禁用） */}
              {!drafting && briefData && (
                <button className="btn btn-ghost btn-sm" onClick={handleDraft} disabled={submitting} title="据当前讨论重新梳理需求">
                  <Icon name="refresh" size={14} />
                  <span>重新梳理</span>
                </button>
              )}
              <button className="btn" style={{ marginLeft: 'auto' }} onClick={onClose} disabled={submitting}>取消</button>
              <button className="btn btn-primary" onClick={handleStart} disabled={submitting || drafting || !title.trim() || !brief.trim()}>
                {submitting ? <Icon name="refresh" size={15} className="spin" /> : <Icon name="zap" size={15} />}
                <span>{submitting ? '创建中…' : '创建需求并开始编码'}</span>
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function Composer({ conv, agents, contextAttachments, onSend, onCompress, onError, quote, onClearQuote, wsRefs, onRemoveWsRef, busy }: {
  conv: Conversation; agents: Agent[]; contextAttachments: ConversationAttachment[];
  onSend: (text: string, attachments: PendingAttachment[], contextRefs: ConversationAttachment[], mentionedAgentIds: string[]) => Promise<boolean>;
  onCompress: (mode: 'summary' | 'conclusion') => Promise<boolean>;
  onError: (message: string) => void;
  quote: QuoteDraft | null;
  onClearQuote: () => void;
  wsRefs: WorkspaceRef[];
  onRemoveWsRef: (path: string) => void;
  busy?: boolean;
}) {
  const [text, setText] = useState('');
  const [pending, setPending] = useState<PendingAttachment[]>([]);
  const [showMention, setShowMention] = useState(false);
  const [showAttachmentPicker, setShowAttachmentPicker] = useState(false);
  const [attachmentQuery, setAttachmentQuery] = useState('');
  const [mentionSel, setMentionSel] = useState(0);
  const [attachmentSel, setAttachmentSel] = useState(0);
  const composerRef = useRef<HTMLDivElement>(null);
  const attachmentPopRef = useRef<HTMLDivElement>(null);
  const attachmentTriggerRef = useRef<HTMLButtonElement>(null);
  const editorRef = useRef<HTMLDivElement>(null);
  // IME（中文/日文）合成态。WebKitGTK(Tauri Linux) 上 KeyboardEvent.isComposing 在「上屏候选词的
  // 那次 Enter」上报不可靠，故自己用 compositionstart/end 维护一份状态，作为 Enter 发送的权威闸门——
  // 否则合成期 Enter 会被当成发送、preventDefault 掐断上屏，正在输入的文字直接丢失（气泡里没有文本）。
  const composingRef = useRef(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const imageInputRef = useRef<HTMLInputElement>(null);
  // 实时语音录入（复用已有 RealtimeAsr / asrRealtime* IPC）：识别结果写进一个专用文本节点，
  // 保留编辑器内已有文本与 @ 标签；committed 为已定句、partial 为当前增量句。
  const [asrRecording, setAsrRecording] = useState(false);
  // 点击后到麦克风/后端就绪前的过渡态，用于立刻给出「连接中…」反馈，消除点击空窗。
  const [asrStarting, setAsrStarting] = useState(false);
  // 会议室「立即编码」确认弹窗开关（仅绑定项目的群聊可用）。
  const [codeNowOpen, setCodeNowOpen] = useState(false);
  // 工作区文件下拉菜单开关
  const [showWsRefMenu, setShowWsRefMenu] = useState(false);
  const wsRefMenuRef = useRef<HTMLDivElement>(null);
  // 快捷操作（总结/结论/待办/立即编码）下拉菜单开关
  const [showQuickMenu, setShowQuickMenu] = useState(false);
  const quickMenuRef = useRef<HTMLDivElement>(null);
  const asrRef = useRef<RealtimeAsr | null>(null);
  const asrNodeRef = useRef<Text | null>(null);
  const asrCommittedRef = useRef('');
  const asrPartialRef = useRef('');
  const isG = conv.conv_type === 'group';
  const agentMap = useMemo(() => Object.fromEntries(agents.map(a => [a.id, a])), [agents]);
  const members  = useMemo(
    () => {
      if (!isG) return [];
      const list = conv.members.map(id => agentMap[id]).filter((a): a is Agent => !!a && a.mentionable && a.enabled);
      // 绑定了项目的群聊：把内置「编码 Agent」显式追加为可 @ 成员（不受其 mentionable=false 影响）。
      if (conv.project_id) list.push(builtinCodeAgent());
      return list;
    },
    [isG, conv.members, conv.project_id, agentMap],
  );
  // 下拉候选：成员超过 1 人时，置顶一个「@所有人」选项。
  const mentionItems = useMemo<MentionItem[]>(() => {
    const list: MentionItem[] = [];
    if (members.length > 1) list.push({ kind: 'all' });
    for (const a of members) list.push({ kind: 'agent', agent: a });
    return list;
  }, [members]);
  const filteredContextAttachments = useMemo(() => {
    const q = attachmentQuery.trim().toLowerCase();
    if (!q) return contextAttachments;
    return contextAttachments.filter(a => a.original_name.toLowerCase().includes(q) || a.mime.toLowerCase().includes(q));
  }, [contextAttachments, attachmentQuery]);
  const visibleContextAttachments = useMemo(
    () => filteredContextAttachments.slice(0, 8),
    [filteredContextAttachments],
  );

  useEffect(() => {
    setAttachmentSel(0);
  }, [attachmentQuery, contextAttachments]);

  useEffect(() => {
    if (!showAttachmentPicker) return;
    const closeOnOutside = (e: PointerEvent) => {
      if (!(e.target instanceof Node)) return;
      if (attachmentPopRef.current?.contains(e.target)) return;
      if (attachmentTriggerRef.current?.contains(e.target)) return;
      setShowAttachmentPicker(false);
      setAttachmentQuery('');
    };
    const closeOnEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setShowAttachmentPicker(false);
        setAttachmentQuery('');
      }
    };
    document.addEventListener('pointerdown', closeOnOutside);
    document.addEventListener('keydown', closeOnEscape);
    return () => {
      document.removeEventListener('pointerdown', closeOnOutside);
      document.removeEventListener('keydown', closeOnEscape);
    };
  }, [showAttachmentPicker]);

  useEffect(() => {
    if (!showWsRefMenu) return;
    const closeOnOutside = (e: PointerEvent) => {
      if (!(e.target instanceof Node)) return;
      if (wsRefMenuRef.current?.contains(e.target)) return;
      setShowWsRefMenu(false);
    };
    const closeOnEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setShowWsRefMenu(false);
    };
    document.addEventListener('pointerdown', closeOnOutside);
    document.addEventListener('keydown', closeOnEscape);
    return () => {
      document.removeEventListener('pointerdown', closeOnOutside);
      document.removeEventListener('keydown', closeOnEscape);
    };
  }, [showWsRefMenu]);

  useEffect(() => {
    if (!showQuickMenu) return;
    const closeOnOutside = (e: PointerEvent) => {
      if (!(e.target instanceof Node)) return;
      if (quickMenuRef.current?.contains(e.target)) return;
      setShowQuickMenu(false);
    };
    const closeOnEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setShowQuickMenu(false);
    };
    document.addEventListener('pointerdown', closeOnOutside);
    document.addEventListener('keydown', closeOnEscape);
    return () => {
      document.removeEventListener('pointerdown', closeOnOutside);
      document.removeEventListener('keydown', closeOnEscape);
    };
  }, [showQuickMenu]);

  const editorText = () => (editorRef.current?.innerText ?? '').replace(/\u00a0/g, ' ');

  const textBeforeCaret = () => {
    const editor = editorRef.current;
    const sel = window.getSelection();
    if (!editor || !sel || sel.rangeCount === 0) return '';
    const range = sel.getRangeAt(0);
    if (!editor.contains(range.startContainer)) return '';
    const before = range.cloneRange();
    before.selectNodeContents(editor);
    before.setEnd(range.startContainer, range.startOffset);
    return before.toString().replace(/\u00a0/g, ' ');
  };

  const setCaretAfter = (node: Node) => {
    const range = document.createRange();
    range.setStartAfter(node);
    range.collapse(true);
    const sel = window.getSelection();
    sel?.removeAllRanges();
    sel?.addRange(range);
  };

  // 把光标放进文本节点「内部」的末尾，而非节点之间的边界。紧跟 contentEditable=false 标签插入文字时，
  // WebKitGTK 对「编辑器元素层的边界光标」会把 IME 合成结果丢弃；落在真实文本节点内部则能稳定追加。
  const setCaretInsideEnd = (node: Text) => {
    const range = document.createRange();
    range.setStart(node, node.length);
    range.collapse(true);
    const sel = window.getSelection();
    sel?.removeAllRanges();
    sel?.addRange(range);
  };

  const setCaretToEnd = () => {
    const editor = editorRef.current;
    if (!editor) return;
    const range = document.createRange();
    range.selectNodeContents(editor);
    range.collapse(false);
    const sel = window.getSelection();
    sel?.removeAllRanges();
    sel?.addRange(range);
  };

  // pending 含 File，无法序列化；用 ref 镜像最新值供卸载时的保存闭包读取。
  const pendingRef = useRef(pending);
  useEffect(() => { pendingRef.current = pending; }, [pending]);

  // 会话切换 / 从其它页面返回时，恢复「该会话自己」的草稿；离开（会话切换或组件卸载）
  // 前把当前草稿存回，保证每个会话窗口的输入内容互不共享、且切换页面不丢失。
  // 用 useLayoutEffect 在绘制前完成 innerHTML 替换，避免短暂闪现上一个会话的内容。
  useLayoutEffect(() => {
    const id = conv.id;
    const draft = loadComposerDraft(id);
    if (editorRef.current) editorRef.current.innerHTML = draft.html;
    setText(editorText());
    setPending(draft.pending);
    setShowMention(false);
    setShowAttachmentPicker(false);
    const timer = setTimeout(() => {
      editorRef.current?.focus();
      setCaretToEnd();
    }, 0);
    return () => {
      clearTimeout(timer);
      saveComposerDraft(id, {
        html: editorRef.current?.innerHTML ?? '',
        pending: pendingRef.current,
      });
    };
  }, [conv.id]);

  const findTextPosition = (editor: HTMLElement, target: number) => {
    const walker = document.createTreeWalker(editor, NodeFilter.SHOW_TEXT);
    let seen = 0;
    let node = walker.nextNode() as Text | null;
    while (node) {
      const len = node.data.length;
      if (seen + len >= target) return { node, offset: Math.max(0, target - seen) };
      seen += len;
      node = walker.nextNode() as Text | null;
    }
    return null;
  };

  const isMentionTag = (node: Node | null): node is HTMLElement =>
    node instanceof HTMLElement && node.classList.contains('mention-tag');

  const isContextAttachmentTag = (node: Node | null): node is HTMLElement =>
    node instanceof HTMLElement && node.classList.contains('context-attachment-tag');

  const isInlineTag = (node: Node | null): node is HTMLElement =>
    isMentionTag(node) || isContextAttachmentTag(node);

  const isBlankText = (node: Node | null): node is Text =>
    node instanceof Text && /^[\s\u00a0]*$/.test(node.data);

  const removeInlineTag = (tag: HTMLElement, spacers: Text[] = []) => {
    const parent = tag.parentNode;
    const index = parent ? Array.prototype.indexOf.call(parent.childNodes, tag) : -1;
    spacers.forEach(spacer => spacer.remove());
    tag.remove();
    if (parent && index >= 0) {
      const range = document.createRange();
      range.setStart(parent, Math.min(index, parent.childNodes.length));
      range.collapse(true);
      const sel = window.getSelection();
      sel?.removeAllRanges();
      sel?.addRange(range);
    }
    setText(editorText());
    return true;
  };

  const removeInlineTagBeforeChildIndex = (editor: HTMLElement, startIndex: number, seedSpacers: Text[] = []) => {
    const spacers = [...seedSpacers];
    for (let i = startIndex; i >= 0; i--) {
      const child = editor.childNodes[i];
      if (isInlineTag(child)) return removeInlineTag(child, spacers);
      if (isBlankText(child)) {
        spacers.push(child);
        continue;
      }
      return false;
    }
    return false;
  };

  const syncEditor = () => {
    const next = editorText();
    setText(next);
    const before = textBeforeCaret();
    const mentionMatch = before.match(/@([^\s@#]*)$/);
    const attachmentMatch = before.match(/#([^\s@#]*)$/);
    if (members.length > 0 && mentionMatch) {
      setShowMention(true);
      setShowAttachmentPicker(false);
      setMentionSel(0);
      return;
    }
    if (attachmentMatch) {
      setAttachmentQuery(attachmentMatch[1] ?? '');
      setShowAttachmentPicker(true);
      setShowMention(false);
      setAttachmentSel(0);
      return;
    }
    setShowMention(false);
    setShowAttachmentPicker(false);
  };

  const insertPlainText = (value: string) => {
    const editor = editorRef.current;
    if (!editor) return;
    editor.focus();
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0 || !editor.contains(sel.getRangeAt(0).startContainer)) {
      setCaretToEnd();
    }
    document.execCommand('insertText', false, value);
    syncEditor();
  };

  // 语音录入：把 getUserMedia / 后端启动失败翻译成对用户友好的中文提示。
  const asrErrorText = (e: unknown): string => {
    const name = (e as { name?: string } | null)?.name;
    if (name === 'NotAllowedError' || name === 'SecurityError') {
      return '麦克风权限被拒绝，请在系统设置中允许 AutoForge 使用麦克风后重试';
    }
    if (name === 'NotFoundError') return '未检测到麦克风设备，请检查录音设备';
    const msg = (e as { message?: string } | null)?.message;
    return '语音识别启动失败：' + String(msg ?? e);
  };

  const stopAsr = async () => {
    setAsrRecording(false);
    setAsrStarting(false);
    const rt = asrRef.current;
    asrRef.current = null;
    asrNodeRef.current = null;
    await rt?.stop();
    setTimeout(() => { editorRef.current?.focus(); setCaretToEnd(); }, 30);
  };

  const startAsr = async () => {
    const editor = editorRef.current;
    if (!editor || busy || asrRecording || asrStarting) return;
    // 立刻进入「连接中」态——在任何 await 前点亮，消除点击后到识别就绪的反馈空窗。
    setAsrStarting(true);
    // 未配置 ASR（无 API Key）→ 友好引导，且不触发麦克风权限弹窗。
    try {
      const cfg = await getAsrSettings();
      if (!cfg.api_key_set) {
        setAsrStarting(false);
        onError('尚未配置语音识别，请前往「设置 → 语音录入」配置 API Key 后再试');
        return;
      }
    } catch (e) {
      setAsrStarting(false);
      onError('读取语音识别配置失败：' + String(e));
      return;
    }
    // 在光标处插入专用文本节点承载识别结果（保留已有文本与 @ 标签）。
    editor.focus();
    let sel = window.getSelection();
    if (!sel || sel.rangeCount === 0 || !editor.contains(sel.getRangeAt(0).startContainer)) setCaretToEnd();
    sel = window.getSelection();
    if (!sel || sel.rangeCount === 0) return;
    const range = sel.getRangeAt(0);
    range.deleteContents();
    const node = document.createTextNode('');
    range.insertNode(node);
    asrNodeRef.current = node;
    asrCommittedRef.current = '';
    asrPartialRef.current = '';
    const rt = new RealtimeAsr();
    asrRef.current = rt;
    onError('');
    try {
      await rt.start((t, isFinal) => {
        if (isFinal) { asrCommittedRef.current += t; asrPartialRef.current = ''; }
        else asrPartialRef.current = t;
        const n = asrNodeRef.current;
        if (!n || !editorRef.current?.contains(n)) return;
        n.data = asrCommittedRef.current + asrPartialRef.current;
        setCaretAfter(n);
        setText(editorText());
      }, () => {
        // 麦克风就绪、开始收音（后端握手可能仍在进行）：立即切到「聆听中」。
        setAsrStarting(false);
        setAsrRecording(true);
      });
    } catch (e) {
      setAsrStarting(false);
      setAsrRecording(false);
      asrRef.current = null;
      asrNodeRef.current = null;
      try { node.remove(); } catch { /* ignore */ }
      onError(asrErrorText(e));
    }
  };

  // 卸载或切换会话时停止录音（已识别文本保留在编辑器草稿中）。
  useEffect(() => () => { if (asrRef.current) void stopAsr(); }, [conv.id]);

  // 把开/关录音逻辑镜像进 ref，供全局语音快捷键调用最新闭包。
  const toggleAsrRef = useRef<() => void>(() => {});
  toggleAsrRef.current = () => {
    if (busy) return;
    if (asrRecording || asrStarting) void stopAsr(); else void startAsr();
  };
  // 登记为活跃语音面：会议室 Composer 挂载时，全局语音快捷键切换它的录音。
  useEffect(() => registerVoiceSurface(() => toggleAsrRef.current()), []);

  const insertMentionTag = (className: string, agentId: string, label: string) => {
    const editor = editorRef.current;
    const sel = window.getSelection();
    if (!editor || !sel || sel.rangeCount === 0) return;
    const range = sel.getRangeAt(0);
    if (!editor.contains(range.startContainer)) return;

    const before = textBeforeCaret();
    const match = before.match(/@[^\s@]*$/);
    if (match) {
      const start = findTextPosition(editor, before.length - match[0].length);
      if (start) {
        range.setStart(start.node, start.offset);
        range.deleteContents();
      }
    }

    const tag = document.createElement('span');
    tag.className = className;
    tag.contentEditable = 'false';
    tag.dataset.agentId = agentId;
    tag.textContent = label;
    const spacer = document.createTextNode('\u00a0');
    range.insertNode(spacer);
    range.insertNode(tag);
    setCaretInsideEnd(spacer);
    setShowMention(false);
    setText(editorText());
    editor.focus();
  };

  const pickMention = (a: Agent) => insertMentionTag('mention-tag', a.id, '@' + a.name);
  const pickAll = () => insertMentionTag('mention-tag mention-all', ALL_MENTION_ID, '@\u6240\u6709\u4eba');
  const pickMentionItem = (item: MentionItem | undefined) => {
    if (!item) return;
    if (item.kind === 'all') pickAll();
    else pickMention(item.agent);
  };

  const pickSlashCommand = (c: SlashCommand) => {
    const editor = editorRef.current;
    if (!editor) return;
    editor.textContent = '';
    setCaretToEnd();
    insertPlainText('/' + c.name + ' ');
  };

  const pickContextAttachment = (a: ConversationAttachment) => {
    const editor = editorRef.current;
    if (!editor) return;
    editor.focus();
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0 || !editor.contains(sel.getRangeAt(0).startContainer)) {
      setCaretToEnd();
    }
    const nextSel = window.getSelection();
    if (!nextSel || nextSel.rangeCount === 0) return;
    const range = nextSel.getRangeAt(0);

    const before = textBeforeCaret();
    const match = before.match(/#[^\s@#]*$/);
    if (match) {
      const start = findTextPosition(editor, before.length - match[0].length);
      if (start) {
        range.setStart(start.node, start.offset);
        range.deleteContents();
      }
    }

    const tag = document.createElement('span');
    tag.className = 'context-attachment-tag';
    tag.contentEditable = 'false';
    tag.dataset.attachmentId = a.id;
    tag.dataset.kind = a.kind;
    tag.textContent = '#' + a.original_name;
    const spacer = document.createTextNode('\u00a0');
    range.insertNode(spacer);
    range.insertNode(tag);
    setCaretInsideEnd(spacer);
    setShowAttachmentPicker(false);
    setAttachmentQuery('');
    setText(editorText());
    editor.focus();
  };

  const contextRefs = () => {
    const editor = editorRef.current;
    if (!editor) return [];
    const ids = Array.from(editor.querySelectorAll<HTMLElement>('.context-attachment-tag'))
      .map(node => node.dataset.attachmentId)
      .filter((id): id is string => !!id);
    const uniqueIds = Array.from(new Set(ids));
    return uniqueIds
      .map(id => contextAttachments.find(a => a.id === id))
      .filter((a): a is ConversationAttachment => !!a);
  };

  const mentionedAgentIds = () => {
    const editor = editorRef.current;
    if (!editor) return [];
    const ids = Array.from(editor.querySelectorAll<HTMLElement>('.mention-tag'))
      .map(node => node.dataset.agentId)
      .filter((id): id is string => !!id);
    // `@所有人` 展开为当前全部可点名成员，但**排除内置「编码 Agent」**——它是重型 CLI，
    // 只应被显式单独 @，不随广播一起触发，避免误烧 token。
    const expanded = ids.flatMap(id =>
      id === ALL_MENTION_ID
        ? members.map(m => m.id).filter(mid => mid !== BUILTIN_CODE_AGENT_ID)
        : [id],
    );
    return Array.from(new Set(expanded));
  };

  const deleteAdjacentInlineTag = () => {
    const editor = editorRef.current;
    const sel = window.getSelection();
    if (!editor || !sel || sel.rangeCount === 0 || !sel.isCollapsed) return false;
    const range = sel.getRangeAt(0);

    if (range.startContainer === editor) {
      return removeInlineTagBeforeChildIndex(editor, range.startOffset - 1);
    }

    let child: Node | null = range.startContainer;
    while (child && child.parentNode !== editor) child = child.parentNode;
    if (!child) return false;

    const childIndex = Array.prototype.indexOf.call(editor.childNodes, child);
    if (childIndex < 0) return false;

    if (child.nodeType === Node.TEXT_NODE) {
      const textNode = child as Text;
      const before = textNode.data.slice(0, range.startOffset);
      const after = textNode.data.slice(range.startOffset);
      if (before && !/^[\s\u00a0]*$/.test(before)) return false;

      const spacers: Text[] = [];
      if (before) {
        if (after) textNode.data = after;
        else spacers.push(textNode);
      }
      return removeInlineTagBeforeChildIndex(editor, childIndex - 1, spacers);
    }

    if (child instanceof HTMLElement && editor.contains(child)) {
      const offset = range.startOffset;
      if (offset > 0) return removeInlineTagBeforeChildIndex(child, offset - 1);
      return removeInlineTagBeforeChildIndex(editor, childIndex - 1);
    }

    return false;
  };

  const send = async () => {
    const outgoing = editorText().trim();
    const pendingItems = [...pending];
    const refs = contextRefs();
    const mentions = mentionedAgentIds();
    if (!outgoing && pendingItems.length === 0 && refs.length === 0 && wsRefs.length === 0) return;
    if (asrRef.current) void stopAsr();
    setText('');
    setPending([]);
    if (editorRef.current) editorRef.current.innerHTML = '';
    setShowMention(false);
    setShowAttachmentPicker(false);
    clearComposerDraft(conv.id);
    await onSend(outgoing, pendingItems, refs, mentions);
  };

  // 快捷 tag：压缩类走 onCompress（生成摘要并压缩上下文），普通类直接发送预设指令。
  // quickState 让被点击的 tag 在执行期间显示 spinner+「正在总结…」，完成后短暂回执「✓ 已完成」。
  const [quickState, setQuickState] = useState<{ label: string; phase: 'run' | 'done' } | null>(null);
  const handleQuick = async (q: QuickPrompt) => {
    if (busy) return;
    if (q.compress) {
      setQuickState({ label: q.label, phase: 'run' });
      const ok = await onCompress(q.compress);
      if (ok) {
        setQuickState({ label: q.label, phase: 'done' });
        setTimeout(() => setQuickState(s => (s && s.label === q.label ? null : s)), 1800);
      } else {
        setQuickState(null);
      }
    } else if (q.prompt) {
      await onSend(q.prompt, [], [], []);
    }
  };

  const onKey = (e: React.KeyboardEvent) => {
    // 中文/日文等 IME 合成期间，Enter/Tab/方向键都属于输入法（上屏候选词、翻页）——
    // 不能在此发送或操作弹窗，否则 preventDefault 会取消合成，导致正在输入的文字丢失。
    // 以自维护的 composingRef 为权威闸门，再叠加浏览器原生 isComposing / keyCode 229 兜底。
    if (composingRef.current || e.nativeEvent.isComposing || e.keyCode === 229) return;
    if (showMention && mentionItems.length > 0) {
      if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); pickMentionItem(mentionItems[mentionSel]); return; }
      if (e.key === 'ArrowDown') { e.preventDefault(); setMentionSel(s => (s + 1) % mentionItems.length); return; }
      if (e.key === 'ArrowUp')   { e.preventDefault(); setMentionSel(s => (s - 1 + mentionItems.length) % mentionItems.length); return; }
    }
    if (showAttachmentPicker) {
      if ((e.key === 'Enter' || e.key === 'Tab') && visibleContextAttachments.length > 0) {
        e.preventDefault();
        pickContextAttachment(visibleContextAttachments[attachmentSel] ?? visibleContextAttachments[0]);
        return;
      }
      if (e.key === 'ArrowDown' && visibleContextAttachments.length > 0) {
        e.preventDefault();
        setAttachmentSel(s => (s + 1) % visibleContextAttachments.length);
        return;
      }
      if (e.key === 'ArrowUp' && visibleContextAttachments.length > 0) {
        e.preventDefault();
        setAttachmentSel(s => (s - 1 + visibleContextAttachments.length) % visibleContextAttachments.length);
        return;
      }
    }
    if (e.key === 'Backspace' && deleteAdjacentInlineTag()) {
      e.preventDefault();
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); }
    if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      const nextBefore = textBeforeCaret() + e.key;
      const mentionMatch = nextBefore.match(/@([^\s@#]*)$/);
      const attachmentMatch = nextBefore.match(/#([^\s@#]*)$/);
      if (members.length > 0 && mentionMatch) {
        setShowMention(true);
        setShowAttachmentPicker(false);
        setMentionSel(0);
      } else if (attachmentMatch) {
        setAttachmentQuery(attachmentMatch[1] ?? '');
        setShowAttachmentPicker(true);
        setShowMention(false);
        setAttachmentSel(0);
      }
    }
  };

  const agentName = isG ? '群聊' : (agents.find(a => conv.members.includes(a.id))?.name ?? 'Agent');
  const pickFiles = (mode: 'file' | 'image') => (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files ?? []);
    e.target.value = '';
    if (files.length === 0) return;
    if (pending.length + files.length > 5) {
      onError('一次最多待发送 5 个附件');
      return;
    }

    const next: PendingAttachment[] = [];
    for (const file of files) {
      const ext = fileExt(file.name);
      const allowed = mode === 'image' ? IMAGE_EXTS.includes(ext) : FILE_EXTS.includes(ext);
      if (!allowed) {
        onError(`不支持的附件类型：${file.name}`);
        return;
      }
      if (file.size <= 0) {
        onError(`附件为空：${file.name}`);
        return;
      }
      if (file.size > MAX_ATTACHMENT_BYTES) {
        onError(`附件超过 10 MB：${file.name}`);
        return;
      }
      next.push({ id: `${Date.now()}-${Math.random().toString(16).slice(2)}`, file, mode });
    }
    setPending(items => [...items, ...next]);
    onError('');
  };
  const removePending = (id: string) => setPending(items => items.filter(item => item.id !== id));

  // Innate 斜杠命令补全：仍在输入命令名（首个 token 内、尚无空格）时给出候选。
  const slashToken = (() => {
    const t = text.trimStart();
    if (!t.startsWith('/') || t.includes(' ') || t.includes('\n')) return null;
    return t.slice(1).toLowerCase();
  })();
  const slashMatches = slashToken !== null
    ? SLASH_COMMANDS.filter(c => c.name.startsWith(slashToken))
    : [];

  return (
    <div ref={composerRef} className="composer">
      {codeNowOpen && (
        <CodeNowModal
          conversationId={conv.id}
          onClose={() => setCodeNowOpen(false)}
          onError={onError}
        />
      )}
      <div className="composer-tools">
        <input ref={fileInputRef} type="file" multiple accept={FILE_ACCEPT} hidden onChange={pickFiles('file')} />
        <input ref={imageInputRef} type="file" multiple accept={IMAGE_ACCEPT} hidden onChange={pickFiles('image')} />
        <button className="icon-btn" title="添加附件" disabled={busy} onClick={() => fileInputRef.current?.click()}><Icon name="paperclip" size={18} /></button>
        <button className="icon-btn" title="添加图片" disabled={busy} onClick={() => imageInputRef.current?.click()}><Icon name="image" size={18} /></button>
        <button
          className={'icon-btn' + (asrRecording || asrStarting ? ' composer-mic-on' : '')}
          title={asrRecording || asrStarting ? '停止语音录入' : '语音输入（边说边转写）'}
          disabled={busy}
          onMouseDown={e => e.stopPropagation()}
          onClick={() => { if (asrRecording || asrStarting) void stopAsr(); else void startAsr(); }}
        >
          <Icon name={asrRecording || asrStarting ? 'pause' : 'mic'} size={18} />
        </button>
        {(asrRecording || asrStarting) && (
          <span className="composer-asr-live">
            <span className="composer-asr-dot" />{asrRecording ? '聆听中…' : '连接中…'}
          </span>
        )}
        {isG && (
          <button className="icon-btn" title="@ 指定 Agent" onClick={() => insertPlainText('@')}>
            <Icon name="at" size={18} />
          </button>
        )}
        <button
          ref={attachmentTriggerRef}
          className="context-attach-trigger"
          title="引用历史附件到会议室上下文"
          disabled={busy}
          onMouseDown={e => e.preventDefault()}
          onClick={() => {
            editorRef.current?.focus();
            setAttachmentQuery('');
            setAttachmentSel(0);
            setShowMention(false);
            setShowAttachmentPicker(v => !v);
          }}
        >
          <Icon name="paperclip" size={14} />
          <span>会议室上下文附件</span>
          <b>{contextAttachments.length}</b>
        </button>
        {isG && (() => {
          // 当前是否有快捷操作在运行/刚完成，用于在收起的触发按钮上回显状态。
          const active = quickState?.phase === 'run';
          const doneNow = quickState?.phase === 'done';
          const runLabel = quickState?.label === '形成结论' ? '正在收敛…' : '正在总结…';
          return (
            <div ref={quickMenuRef} style={{ position: 'relative' }}>
              <button
                type="button"
                className={'composer-quick-tag' + (active ? ' active' : '') + (doneNow ? ' done' : '')}
                title="快捷操作：总结内容 / 形成结论 / 列出待办 / 立即编码"
                disabled={busy}
                onMouseDown={e => e.preventDefault()}
                onClick={() => setShowQuickMenu(v => !v)}
              >
                {active
                  ? <Icon name="refresh" size={13} className="spin" />
                  : doneNow
                    ? <Icon name="check" size={13} />
                    : <Icon name="zap" size={13} />}
                <span>{active ? runLabel : doneNow ? '已完成' : '快捷操作'}</span>
                <Icon name="chevron" size={12} style={{ opacity: 0.6 }} />
              </button>
              {showQuickMenu && (
                <div className="mention-pop" style={{ bottom: '100%', marginBottom: 4 }}>
                  <div className="mention-pop-label">快捷操作</div>
                  {QUICK_PROMPTS.map(q => (
                    <div
                      key={q.label}
                      className="mention-row"
                      onMouseDown={e => e.preventDefault()}
                      onClick={() => { setShowQuickMenu(false); void handleQuick(q); }}
                      title={q.compress
                        ? (q.compress === 'conclusion' ? '收敛结论并压缩上下文（历史消息移出后续上下文）' : '总结内容并压缩上下文（历史消息移出后续上下文）')
                        : q.prompt}
                    >
                      <div className="attachment-row-ic"><Icon name={q.icon} size={15} /></div>
                      <div style={{ minWidth: 0 }}>
                        <div className="nm">{q.label}</div>
                        {q.compress && <div className="rl">同时压缩上下文</div>}
                      </div>
                    </div>
                  ))}
                  {conv.project_id && (
                    <div
                      className="mention-row"
                      onMouseDown={e => e.preventDefault()}
                      onClick={() => { setShowQuickMenu(false); setCodeNowOpen(true); }}
                      title="据本次讨论梳理功能点，自动创建需求并立即开始编码（跳过需求审核，仍走代码审核）"
                    >
                      <div className="attachment-row-ic"><Icon name="zap" size={15} /></div>
                      <div style={{ minWidth: 0 }}>
                        <div className="nm">立即编码</div>
                        <div className="rl">梳理功能点并跳过需求审核直接编码</div>
                      </div>
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })()}
        {pending.map(item => (
          <div key={item.id} className="composer-pending-item">
            <Icon name={item.mode === 'image' ? 'image' : 'file'} size={15} />
            <span className="pending-name">{item.file.name}</span>
            <span className="pending-size">{formatBytes(item.file.size)}</span>
            <button className="icon-btn" title="移除附件" disabled={busy} onClick={() => removePending(item.id)}>
              <Icon name="x" size={13} />
            </button>
          </div>
        ))}
        {wsRefs.length > 0 && (
          <div ref={wsRefMenuRef} style={{ position: 'relative' }}>
            <button
              type="button"
              className="composer-quick-tag"
              title={`已引用 ${wsRefs.length} 个工作区文件`}
              disabled={busy}
              onClick={() => setShowWsRefMenu(!showWsRefMenu)}
              style={{ borderColor: 'var(--ember)', color: 'var(--ember)' }}
            >
              <Icon name="folder" size={13} />
              <span>引用 {wsRefs.length}</span>
            </button>
            {showWsRefMenu && (
              <div className="mention-pop" style={{ right: 0, left: 'auto', bottom: '100%', marginBottom: 4 }}>
                <div className="mention-pop-label">已引用的工作区文件</div>
                {wsRefs.map(ref => (
                  <div key={ref.path} className="mention-row" style={{ cursor: 'auto' }}>
                    <Icon name="folder" size={12} style={{ color: 'var(--ember)', marginRight: 4 }} />
                    <div style={{ minWidth: 0, flex: 1 }}>
                      <div className="nm">{ref.name}</div>
                      <div className="rl" style={{ fontSize: 'var(--text-caption)' }}>{ref.path}</div>
                    </div>
                    <button
                      className="icon-btn"
                      style={{ width: 24, height: 24, marginRight: -8 }}
                      title="移除引用"
                      disabled={busy}
                      onClick={() => onRemoveWsRef(ref.path)}
                    >
                      <Icon name="x" size={12} />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
        <div style={{ marginLeft: 'auto', fontSize: 'var(--text-caption)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', paddingRight: 4 }}>
          {isG ? '群聊共享上下文' : 'Enter 发送'}
        </div>
      </div>
      {quote && (
        <div className="composer-quote" title={`${quote.author}: ${quote.text}`}>
          <Icon name="quote" size={15} />
          <span className="quote-author">{quote.author}</span>
          <span className="quote-text">{quote.text}</span>
          <button className="icon-btn" title="取消引用" disabled={busy} onClick={onClearQuote}>
            <Icon name="x" size={13} />
          </button>
        </div>
      )}
      <div className="composer-box" style={{ position: 'relative' }}>
        {slashMatches.length > 0 && !showMention && !showAttachmentPicker && (
          <div className="mention-pop">
            <div className="mention-pop-label">Innate 知识库命令</div>
            {slashMatches.map(c => (
              <div key={c.name} className="mention-row"
                onMouseDown={e => e.preventDefault()}
                onClick={() => pickSlashCommand(c)}>
                <div className="attachment-row-ic"><Icon name={c.icon} size={15} /></div>
                <div style={{ minWidth: 0 }}>
                  <div className="nm" style={{ fontFamily: 'var(--font-mono)' }}>{c.usage}</div>
                  <div className="rl">{c.desc}</div>
                </div>
              </div>
            ))}
          </div>
        )}
        {showMention && mentionItems.length > 0 && (
          <div className="mention-pop">
            <div className="mention-pop-label">@ 指定 Agent 回答</div>
            {mentionItems.map((item, i) => item.kind === 'all' ? (
              <div key="__all__" className={'mention-row' + (i === mentionSel ? ' mention-active' : '')}
                onMouseDown={e => e.preventDefault()}
                onMouseEnter={() => setMentionSel(i)} onClick={() => pickAll()}>
                <div className="mention-all-ic"><Icon name="users" size={16} /></div>
                <div><div className="nm">所有人</div><div className="rl">全员一起讨论，互相分析、@ 彼此尽快达成一致</div></div>
              </div>
            ) : (
              <div key={item.agent.id} className={'mention-row' + (i === mentionSel ? ' mention-active' : '')}
                onMouseDown={e => e.preventDefault()}
                onMouseEnter={() => setMentionSel(i)} onClick={() => pickMention(item.agent)}>
                <Avatar agent={item.agent} size={30} />
                <div><div className="nm">{item.agent.name}</div><div className="rl">{item.agent.role}</div></div>
              </div>
            ))}
          </div>
        )}
        {showAttachmentPicker && (
          <div ref={attachmentPopRef} className="mention-pop attachment-pop">
            <div className="mention-pop-label"># 引用会议室上下文附件</div>
            {visibleContextAttachments.length > 0 ? (
              visibleContextAttachments.map((a, i) => (
                <div
                  key={a.id}
                  className={'mention-row attachment-row' + (i === attachmentSel ? ' mention-active' : '')}
                  onMouseDown={e => e.preventDefault()}
                  onMouseEnter={() => setAttachmentSel(i)}
                  onClick={() => pickContextAttachment(a)}
                >
                  <div className="attachment-row-ic">
                    <Icon name={a.kind === 'image' ? 'image' : 'file'} size={15} />
                  </div>
                  <div style={{ minWidth: 0 }}>
                    <div className="nm">{a.original_name}</div>
                    <div className="rl">{a.mime} · {formatBytes(a.size_bytes)}</div>
                  </div>
                </div>
              ))
            ) : (
              <div className="attachment-empty">暂无可引用附件</div>
            )}
          </div>
        )}
        <div
          ref={editorRef}
          className="composer-editor"
          contentEditable={!busy}
          suppressContentEditableWarning
          data-placeholder={isG ? '输入消息，@ 可指定 Agent 回答…' : `给 ${agentName} 发消息…`}
          onInput={syncEditor}
          onCompositionStart={() => { composingRef.current = true; }}
          onCompositionEnd={() => { composingRef.current = false; syncEditor(); }}
          onKeyDown={onKey}
          onPaste={e => {
            e.preventDefault();
            insertPlainText(e.clipboardData.getData('text/plain'));
          }}
        />
        <button className="send-btn" disabled={(!text.trim() && pending.length === 0 && wsRefs.length === 0) || busy} onClick={send}>
          <Icon name="send" size={18} />
        </button>
      </div>
    </div>
  );
}

function NewGroupModal({ agents, projects, onClose, onCreate }: {
  agents: Agent[];
  projects: Project[];
  onClose: () => void;
  onCreate: (name: string, ids: string[], projectId: string | null) => void;
}) {
  const [sel, setSel] = useState<string[]>([]);
  const [name, setName] = useState('');
  const [projectId, setProjectId] = useState<string>('');
  const [projOpen, setProjOpen] = useState(false);
  const projRef = useRef<HTMLDivElement>(null);
  const chatAgents = useMemo(
    () => agents.filter(a => a.visible_in_chat && a.mentionable && a.enabled),
    [agents],
  );
  const toggle = (id: string) => setSel(s => s.includes(id) ? s.filter(x => x !== id) : [...s, id]);
  const selectedProject = projects.find(p => p.id === projectId) ?? null;

  useEffect(() => {
    if (!projOpen) return;
    const close = (e: PointerEvent) => {
      if (projRef.current && !projRef.current.contains(e.target as Node)) setProjOpen(false);
    };
    document.addEventListener('pointerdown', close);
    return () => document.removeEventListener('pointerdown', close);
  }, [projOpen]);

  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 200 }}>
      <div style={{ width: 440, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center' }}>
          <div>
            <div className="eyebrow" style={{ fontSize: 'var(--text-section)' }}><span className="cn">新建群聊</span></div>
            <div style={{ fontSize: 'var(--text-control)', color: 'var(--text-3)', marginTop: 4 }}>拉多个 Agent 进入群聊，共享上下文协同讨论需求</div>
          </div>
          <button className="icon-btn" style={{ marginLeft: 'auto' }} onClick={onClose}><Icon name="x" size={18} /></button>
        </div>
        <div style={{ padding: '16px 20px' }}>
          <div className="field" style={{ marginBottom: 14 }}>
            <label>群聊名称</label>
            <input value={name} onChange={e => setName(e.target.value)} placeholder="例如：Vocant · 导出性能优化" />
          </div>
          <div className="field" style={{ marginBottom: 14 }}>
            <label>绑定项目（可选）</label>
            <div style={{ position: 'relative' }} ref={projRef}>
              <div
                className="proj-select"
                style={{ padding: '8px 12px', borderRadius: 10 }}
                onClick={() => setProjOpen(o => !o)}
              >
                {selectedProject ? (
                  <>
                    <div className="proj-logo" style={{ background: 'var(--ember)', width: 28, height: 28, fontSize: 'var(--text-label)', borderRadius: 8 }}>
                      {selectedProject.name[0]}
                    </div>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div className="proj-name" style={{ fontSize: 'var(--text-control)' }}>{selectedProject.name}</div>
                      <div className="proj-meta">{selectedProject.description || selectedProject.slug}</div>
                    </div>
                  </>
                ) : (
                  <div style={{ flex: 1, fontSize: 'var(--text-control)', color: 'var(--text-3)' }}>不绑定项目（通用群聊）</div>
                )}
                <Icon name="chevDown" size={15} style={{ color: 'var(--text-3)', transition: 'transform .15s', transform: projOpen ? 'rotate(180deg)' : 'none', flexShrink: 0 }} />
              </div>
              {projOpen && (
                <div className="mention-pop" style={{ left: 0, right: 0, top: 'calc(100% + 5px)', bottom: 'auto', width: '100%', maxHeight: 200, overflowY: 'auto', zIndex: 300 }}>
                  <div
                    className="mention-row"
                    onClick={() => { setProjectId(''); setProjOpen(false); }}
                  >
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div className="nm">不绑定项目</div>
                      <div className="rl">通用群聊，无项目上下文</div>
                    </div>
                    {!projectId && <Icon name="check" size={13} style={{ color: 'var(--ember)', flexShrink: 0 }} />}
                  </div>
                  {projects.map(p => (
                    <div
                      key={p.id}
                      className="mention-row"
                      onClick={() => { setProjectId(p.id); setProjOpen(false); }}
                    >
                      <div className="proj-logo" style={{ background: 'var(--ember)', width: 26, height: 26, fontSize: 'var(--text-caption)', borderRadius: 7, flexShrink: 0 }}>
                        {p.name[0]}
                      </div>
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div className="nm">{p.name}</div>
                        <div className="rl">{p.description || p.slug}</div>
                      </div>
                      {projectId === p.id && <Icon name="check" size={13} style={{ color: 'var(--ember)', flexShrink: 0 }} />}
                    </div>
                  ))}
                </div>
              )}
            </div>
            {projectId && (
              <div style={{ fontSize: 'var(--text-label)', color: 'var(--ember)', display: 'flex', alignItems: 'center', gap: 4 }}>
                <Icon name="zap" size={11} />
                claude.md / agents.md 将自动注入每次对话上下文
              </div>
            )}
          </div>
          <div className="field"><label>选择 Agent（{sel.length}）</label></div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6, marginTop: 8, maxHeight: 240, overflowY: 'auto' }}>
            {chatAgents.map(a => (
              <div key={a.id} className="mention-row"
                style={{ border: '1px solid ' + (sel.includes(a.id) ? 'var(--ember)' : 'transparent'), background: sel.includes(a.id) ? 'var(--ember-tint)' : 'transparent' }}
                onClick={() => toggle(a.id)}>
                <Avatar agent={a} size={34} />
                <div style={{ flex: 1 }}><div className="nm">{a.name}</div><div className="rl">{a.role}</div></div>
                {sel.includes(a.id) && <Icon name="check" size={14} style={{ color: 'var(--ember)' }} />}
              </div>
            ))}
          </div>
        </div>
        <div style={{ padding: '14px 20px', borderTop: '1px solid var(--border)', display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn btn-primary" disabled={sel.length < 2 || !name.trim()} onClick={() => onCreate(name, sel, projectId || null)}>
            <Icon name="plus" size={15} />创建群聊
          </button>
        </div>
      </div>
    </div>
  );
}

function EditGroupModal({ conversation, projects, onClose, onSave }: {
  conversation: Conversation;
  projects: Project[];
  onClose: () => void;
  onSave: (conversationId: string, name: string, projectId: string | null) => void;
}) {
  const [name, setName] = useState(conversation.name ?? '');
  const [projectId, setProjectId] = useState(conversation.project_id ?? '');
  const [projOpen, setProjOpen] = useState(false);
  const projRef = useRef<HTMLDivElement>(null);
  const selectedProject = projects.find(p => p.id === projectId) ?? null;
  const dirty = name.trim() !== (conversation.name ?? '') || (projectId || null) !== (conversation.project_id ?? null);

  useEffect(() => {
    if (!projOpen) return;
    const close = (e: PointerEvent) => {
      if (projRef.current && !projRef.current.contains(e.target as Node)) setProjOpen(false);
    };
    document.addEventListener('pointerdown', close);
    return () => document.removeEventListener('pointerdown', close);
  }, [projOpen]);

  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 200 }}>
      <div style={{ width: 440, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center' }}>
          <div>
            <div className="eyebrow" style={{ fontSize: 'var(--text-section)' }}><span className="cn">编辑会议室</span></div>
            <div style={{ fontSize: 'var(--text-control)', color: 'var(--text-3)', marginTop: 4 }}>修改群名称和项目绑定信息</div>
          </div>
          <button className="icon-btn" style={{ marginLeft: 'auto' }} onClick={onClose}><Icon name="x" size={18} /></button>
        </div>
        <div style={{ padding: '16px 20px' }}>
          <div className="field" style={{ marginBottom: 14 }}>
            <label>群聊名称</label>
            <input value={name} onChange={e => setName(e.target.value)} placeholder="例如：Vocant · 导出性能优化" />
          </div>
          <div className="field" style={{ marginBottom: 14 }}>
            <label>绑定项目（可选）</label>
            <div style={{ position: 'relative' }} ref={projRef}>
              <div
                className="proj-select"
                style={{ padding: '8px 12px', borderRadius: 10 }}
                onClick={() => setProjOpen(o => !o)}
              >
                {selectedProject ? (
                  <>
                    <div className="proj-logo" style={{ background: 'var(--ember)', width: 28, height: 28, fontSize: 'var(--text-label)', borderRadius: 8 }}>
                      {selectedProject.name[0]}
                    </div>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div className="proj-name" style={{ fontSize: 'var(--text-control)' }}>{selectedProject.name}</div>
                      <div className="proj-meta">{selectedProject.description || selectedProject.slug}</div>
                    </div>
                  </>
                ) : (
                  <div style={{ flex: 1, fontSize: 'var(--text-control)', color: 'var(--text-3)' }}>不绑定项目（通用群聊）</div>
                )}
                <Icon name="chevDown" size={15} style={{ color: 'var(--text-3)', transition: 'transform .15s', transform: projOpen ? 'rotate(180deg)' : 'none', flexShrink: 0 }} />
              </div>
              {projOpen && (
                <div className="mention-pop" style={{ left: 0, right: 0, top: 'calc(100% + 5px)', bottom: 'auto', width: '100%', maxHeight: 200, overflowY: 'auto', zIndex: 300 }}>
                  <div
                    className="mention-row"
                    onClick={() => { setProjectId(''); setProjOpen(false); }}
                  >
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div className="nm">不绑定项目</div>
                      <div className="rl">通用群聊，无项目上下文</div>
                    </div>
                    {!projectId && <Icon name="check" size={13} style={{ color: 'var(--ember)', flexShrink: 0 }} />}
                  </div>
                  {projects.map(p => (
                    <div
                      key={p.id}
                      className="mention-row"
                      onClick={() => { setProjectId(p.id); setProjOpen(false); }}
                    >
                      <div className="proj-logo" style={{ background: 'var(--ember)', width: 26, height: 26, fontSize: 'var(--text-caption)', borderRadius: 7, flexShrink: 0 }}>
                        {p.name[0]}
                      </div>
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div className="nm">{p.name}</div>
                        <div className="rl">{p.description || p.slug}</div>
                      </div>
                      {projectId === p.id && <Icon name="check" size={13} style={{ color: 'var(--ember)', flexShrink: 0 }} />}
                    </div>
                  ))}
                </div>
              )}
            </div>
            {conversation.project_id !== (projectId || null) && (
              <div style={{ fontSize: 'var(--text-label)', color: 'var(--amber)', display: 'flex', alignItems: 'center', gap: 4 }}>
                <Icon name="alert" size={11} />
                修改项目绑定会清空已固定的项目上下文文件
              </div>
            )}
          </div>
        </div>
        <div style={{ padding: '14px 20px', borderTop: '1px solid var(--border)', display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn btn-primary" disabled={!name.trim() || !dirty} onClick={() => onSave(conversation.id, name, projectId || null)}>
            <Icon name="check" size={15} />保存
          </button>
        </div>
      </div>
    </div>
  );
}

// 只读归档消息行：用快照里去规范化的发言人信息渲染，不依赖 live agents，无右键菜单/编辑。
function ArchiveMessageRow({ m, agents, isGroup, projectId, highlight }: {
  m: ArchivedMessage; agents: Agent[]; isGroup: boolean; projectId?: string; highlight?: string;
}) {
  const agentMap = useMemo(() => Object.fromEntries(agents.map(a => [a.id, a])), [agents]);
  const liveAgent = !m.is_me && !m.is_innate && m.from_agent ? agentMap[m.from_agent] : null;
  const pseudo: Message = {
    id: '', conversation_id: '', from_agent: m.from_agent,
    content_json: m.content_json, created_at: m.created_at,
    excluded_from_context: m.excluded_from_context,
  };
  const blocks = visibleMessageBlocks(pseudo);
  const quote = messageQuote(pseudo);
  // Agent/Innate 回复一律用「文档流」（bubble doc）统一呈现；「我」的消息保持气泡右对齐。
  const longDoc = !m.is_me;
  const bubbleRef = useRef<HTMLDivElement>(null);
  const headings = useMemo(() => (longDoc ? docHeadings(blocks) : []), [longDoc, blocks]);
  const showToc = headings.length >= 3;
  const jumpToHeading = (i: number) => {
    bubbleRef.current?.querySelectorAll('h1,h2,h3')[i]?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };
  return (
    <div className={'msg' + (m.is_me ? ' me' : '') + (longDoc ? ' msg-doc' : '')}>
      {m.is_me
        ? <MeAvatar size={36} />
        : m.is_innate
          ? <div className="av" style={{ width: 36, height: 36, background: 'var(--ember-tint-strong)', color: 'var(--ember-soft)', display: 'flex', alignItems: 'center', justifyContent: 'center' }} title="Innate 知识库"><Icon name="brain" size={20} /></div>
          : liveAgent
            ? <Avatar agent={liveAgent} size={36} />
            : <div className="av" style={{ width: 36, height: 36, background: m.author_color || 'var(--bg-3)', color: 'var(--text)', fontSize: 'var(--text-body)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>{m.author.slice(0, 1)}</div>}
      <div className="msg-body">
        {!m.is_me && (
          <div className="msg-meta">
            <span className="msg-author" style={{ color: m.is_innate ? 'var(--ember)' : (m.author_color || 'var(--text)') }}>{m.author}</span>
            {m.is_innate
              ? <span className="chip ember" style={{ padding: '0px 6px', fontSize: 'var(--text-micro)' }}>KNOWLEDGE</span>
              : isGroup && m.author_en && <span className="chip" style={{ padding: '0px 6px', fontSize: 'var(--text-micro)' }}>{m.author_en}</span>}
            <span className="msg-time" title={fmtFull(m.created_at)}>{fmtMsgTime(m.created_at)}</span>
          </div>
        )}
        <div ref={bubbleRef} className={'bubble' + (longDoc ? ' doc' : '')} style={m.excluded_from_context ? { opacity: 0.45, outline: '1.5px dashed var(--border-strong)', outlineOffset: 2 } : undefined}>
          {blocks.map((b, i) => <Block key={i} b={b} projectId={projectId} highlight={highlight} />)}
        </div>
        {quote && (
          <div className="bubble-quote" title={`${quote.author}: ${quote.text}`}>
            <span className="quote-author">{quote.author}</span>
            <span className="quote-text">{quote.text}</span>
          </div>
        )}
      </div>
      {showToc && <DocToc headings={headings} onJump={jumpToHeading} />}
    </div>
  );
}

// 归档区：左侧检索/列表，右侧只读回顾选中归档的完整对话快照。
function ArchiveBrowser({ agents, projects, onClose }: {
  agents: Agent[]; projects: Project[]; onClose: () => void;
}) {
  const [q, setQ] = useState('');
  const [summaries, setSummaries] = useState<ConversationArchiveSummary[]>([]);
  const [hits, setHits] = useState<ArchiveSearchHit[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ArchivedMessage[]>([]);
  const [selectedMeta, setSelectedMeta] = useState<ConversationArchiveSummary | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<ConversationArchiveSummary | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState('');
  const [expanded, setExpanded] = useState(false);
  const bodyRef = useRef<HTMLDivElement>(null);

  const refreshList = useCallback(async () => {
    try { setSummaries(await listConversationArchives()); }
    catch (e) { setErr(String(e)); }
  }, []);

  useEffect(() => { refreshList(); }, [refreshList]);

  useEffect(() => {
    const term = q.trim();
    if (!term) { setHits([]); return; }
    let cancelled = false;
    const t = setTimeout(async () => {
      try { const r = await searchConversationArchives(term); if (!cancelled) setHits(r); }
      catch (e) { if (!cancelled) setErr(String(e)); }
    }, 180);
    return () => { cancelled = true; clearTimeout(t); };
  }, [q]);

  const openArchive = async (s: ConversationArchiveSummary) => {
    setSelectedId(s.id);
    setSelectedMeta(s);
    setLoading(true);
    setErr('');
    try {
      const detail = await getConversationArchive(s.id);
      const parsed: ArchivedMessage[] = JSON.parse(detail.payload_json);
      setMessages(parsed);
    } catch (e) {
      setErr(String(e));
      setMessages([]);
    } finally {
      setLoading(false);
    }
  };

  const removeArchive = async (id: string) => {
    try {
      await deleteConversationArchive(id);
      if (selectedId === id) { setSelectedId(null); setMessages([]); setSelectedMeta(null); }
      setConfirmDelete(null);
      await refreshList();
    } catch (e) { setErr(String(e)); setConfirmDelete(null); }
  };

  const searching = q.trim().length > 0;
  // 选中归档加载完且有搜索词时，自动滚动到右侧正文第一个高亮命中处。
  useEffect(() => {
    if (loading || !searching || messages.length === 0) return;
    const t = setTimeout(() => {
      bodyRef.current?.querySelector('.search-mark')?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    }, 60);
    return () => clearTimeout(t);
  }, [loading, searching, messages, q]);

  const rows: (ConversationArchiveSummary & { match_count?: number; snippet?: string })[] =
    searching ? hits : summaries;
  const projectName = (s: ConversationArchiveSummary) =>
    s.project_name ?? (s.project_id ? projects.find(p => p.id === s.project_id)?.name : undefined);
  const isGroupMeta = (selectedMeta?.conv_type ?? 'group') === 'group';

  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 240 }}>
      <div style={{ background: 'var(--bg-1)', border: '1px solid var(--border-strong)', borderRadius: 14, width: expanded ? '100%' : 'min(1180px, 94vw)', height: expanded ? '100%' : 'min(820px, 92vh)', display: 'flex', flexDirection: 'column', overflow: 'hidden', boxShadow: 'var(--shadow-lg)', transition: 'width .16s, height .16s' }} onClick={e => e.stopPropagation()}>
        <div className="panel-head" style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '13px 16px', borderBottom: '1px solid var(--border)' }}>
          <Icon name="inbox" size={18} style={{ color: 'var(--ember)' }} />
          <div style={{ flex: 1 }}>
            <div style={{ fontFamily: 'var(--font-display)', fontSize: 'var(--text-section)', fontWeight: 700 }}>归档区</div>
            <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>READ-ONLY · 只读快照检索与回顾</div>
          </div>
          <button className="icon-btn" title={expanded ? '还原窗口' : '放大窗口 · 沉浸阅读'} onClick={() => setExpanded(e => !e)}>
            <Icon name={expanded ? 'minimize' : 'maximize'} size={16} />
          </button>
          <button className="icon-btn" title="关闭" onClick={onClose}><Icon name="x" size={16} /></button>
        </div>
        {err && <div style={{ padding: '6px 16px', color: 'var(--red)', fontSize: 'var(--text-label)' }}>{err}</div>}
        <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
          {/* 左：检索 + 列表 */}
          <div style={{ width: 320, borderRight: '1px solid var(--border)', display: 'flex', flexDirection: 'column', minHeight: 0, background: 'var(--bg-1)' }}>
            <div style={{ padding: 12 }}>
              <div className="search">
                <Icon name="search" size={15} />
                <input placeholder="搜索归档内容或标题…" value={q} onChange={e => setQ(e.target.value)} autoFocus />
              </div>
            </div>
            <div className="scroll" style={{ flex: 1, overflowY: 'auto', padding: '0 8px 10px' }}>
              {rows.length === 0 ? (
                <div className="empty" style={{ padding: '28px 12px', textAlign: 'center', color: 'var(--text-3)', fontSize: 'var(--text-label)' }}>
                  {searching ? '没有匹配的归档' : '暂无归档对话'}
                </div>
              ) : rows.map(s => (
                <div
                  key={s.id}
                  className={'mention-row' + (selectedId === s.id ? ' mention-active' : '')}
                  style={{ alignItems: 'flex-start', flexDirection: 'column', gap: 4, padding: '9px 10px' }}
                  onClick={() => openArchive(s)}
                >
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6, width: '100%' }}>
                    <Icon name="package" size={13} style={{ color: 'var(--ember)', flexShrink: 0 }} />
                    <span className="nm" style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{s.title}</span>
                    <span style={{ fontSize: 'var(--text-micro)', color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', flexShrink: 0 }}>{s.message_count}条</span>
                  </div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6, width: '100%', fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>
                    <span className="chip" style={{ padding: '0 6px', fontSize: 'var(--text-micro)' }}>{s.conv_type === 'group' ? '群聊' : '单聊'}</span>
                    {projectName(s) && <span style={{ color: 'var(--ember)', display: 'inline-flex', alignItems: 'center', gap: 2 }}><Icon name="folder" size={9} />{projectName(s)}</span>}
                    <span style={{ marginLeft: 'auto', fontFamily: 'var(--font-mono)' }}>{fmtListTime(s.archived_at)}</span>
                  </div>
                  {searching && typeof s.match_count === 'number' && (
                    <div style={{ width: '100%', fontSize: 'var(--text-caption)', color: 'var(--text-2)' }}>
                      <span className="chip ember" style={{ padding: '0 6px', fontSize: 'var(--text-micro)' }}>命中 {s.match_count}</span>
                      {s.snippet && <span style={{ marginLeft: 6, color: 'var(--text-3)' }}>{s.snippet}</span>}
                    </div>
                  )}
                </div>
              ))}
            </div>
          </div>
          {/* 右：只读回顾 */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0, background: 'var(--bg)' }}>
            {selectedMeta ? (
              <>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '11px 16px', borderBottom: '1px solid var(--border)' }}>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
                      <Icon name="key" size={13} style={{ color: 'var(--text-3)' }} />
                      <span style={{ fontWeight: 700, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{selectedMeta.title}</span>
                      <span className="chip" style={{ padding: '0 6px', fontSize: 'var(--text-micro)' }}>只读</span>
                    </div>
                    <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)', marginTop: 2 }}>
                      {selectedMeta.message_count} 条 · 归档于 {fmtFull(selectedMeta.archived_at)}
                    </div>
                  </div>
                  <button className="btn btn-sm btn-danger" onClick={() => setConfirmDelete(selectedMeta)} title="彻底删除此归档">
                    <Icon name="trash" size={14} /> 彻底删除
                  </button>
                </div>
                <div ref={bodyRef} className="msgs scroll" style={{ flex: 1 }}>
                  {loading
                    ? <div className="empty" style={{ color: 'var(--text-3)', padding: 30 }}>加载中…</div>
                    : messages.map((m, i) => (
                        <ArchiveMessageRow key={i} m={m} agents={agents} isGroup={isGroupMeta} projectId={selectedMeta.project_id ?? undefined} highlight={searching ? q.trim() : ''} />
                      ))}
                </div>
              </>
            ) : (
              <div className="empty" style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: 10, color: 'var(--text-3)' }}>
                <Icon name="inbox" size={34} style={{ color: 'var(--text-faint)' }} />
                <div style={{ fontSize: 'var(--text-label)' }}>从左侧选择一个归档，只读回顾完整对话</div>
              </div>
            )}
          </div>
        </div>
      </div>
      {confirmDelete && (
        <ConfirmModal
          zIndex={260}
          msg={`确认彻底删除归档「${confirmDelete.title}」（${confirmDelete.message_count} 条）？删除后该归档快照不可恢复。`}
          okLabel="彻底删除"
          onOk={() => removeArchive(confirmDelete.id)}
          onCancel={() => setConfirmDelete(null)}
        />
      )}
    </div>
  );
}

function ConfirmModal({ msg, okLabel, onOk, onCancel, zIndex = 220 }: {
  msg: string; okLabel: string; onOk: () => void; onCancel: () => void; zIndex?: number;
}) {
  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex }}>
      <div style={{ background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 14, padding: '22px 24px', width: 380, boxShadow: 'var(--shadow-lg)' }} onClick={e => e.stopPropagation()}>
        <p style={{ margin: '0 0 20px', fontSize: 'var(--text-body)', lineHeight: 'var(--leading-relaxed)' }}>{msg}</p>
        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onCancel}>取消</button>
          <button className="btn btn-danger" onClick={onOk}>{okLabel}</button>
        </div>
      </div>
    </div>
  );
}

// ─── Main page ────────────────────────────────────────────────────────────────

export default function ConversationsPage() {
  const [convs,          setConvs]          = useState<Conversation[]>([]);
  const [agents,         setAgents]         = useState<Agent[]>([]);
  const [projects,       setProjects]       = useState<Project[]>([]);
  // 会议室是「条件渲染」页面：切到其它页时整个组件卸载。把当前会话 id 持久化到
//  sessionStorage，回到会议室时落回同一个会话（而非默认首个群聊），保证后台仍在
//  进行的对话回到前端就能看见。
  const [active,         setActive]         = useState(() => {
    try { return sessionStorage.getItem('AutoForge:active-conv') || ''; } catch { return ''; }
  });
  const [msgs,           setMsgs]           = useState<Message[]>([]);
  const [showNew,        setShowNew]        = useState(false);
  const [editGroup,      setEditGroup]      = useState<Conversation | null>(null);
  const [showMembers,    setShowMembers]    = useState(false);
  const [showContext,    setShowContext]     = useState(false);
  const [showSearch,     setShowSearch]     = useState(false);
  const [showHeadMore,   setShowHeadMore]   = useState(false);
  const [idCopied,       setIdCopied]       = useState(false);
  const [projectFiles,   setProjectFiles]   = useState<ProjectContextFile[]>([]);
  const [workspaceFiles, setWorkspaceFiles] = useState<WorkspaceFile[]>([]);
  const [workspaceTab,   setWorkspaceTab]   = useState<'docs' | 'specs' | 'deliverables'>('docs');
  const [wsRefs,         setWsRefs]         = useState<WorkspaceRef[]>([]);
  const [searchQuery,    setSearchQuery]    = useState('');
  const [activeSearchId, setActiveSearchId] = useState<string | null>(null);
  // 当前正在看的气泡对应的 agent（跨越视口顶边的那条消息的作者）→ 在堆叠里高亮其边框，
  // 充当「黏性标题」指示当前段落属于谁。
  const [currentAgentId, setCurrentAgentId] = useState<string | null>(null);
  // 堆叠作用域：以「我的发言」分段，当前滚动位置所在那一段里发过言的 agent——按**发言先后**排序的有序数组
  // （非成员名单顺序）。即「只显示到我最近一次发言为止」：最新一轮几个 agent 回复就显示这几个，
  // 顺序与对话流一致。群聊里消息行不再各自显示头像，统一由此堆叠展示（currentAgentId 高亮当前）。
  const [segmentAgents, setSegmentAgents] = useState<string[]>([]);
  // 本段起始处「我的发言」消息 id——堆叠顶部的「我」按钮据此跳转回本轮我的提问。
  const [segmentStartId, setSegmentStartId] = useState<string | null>(null);
  const [confirmDissolve,setConfirmDissolve]= useState<string | null>(null);
  const [confirmArchive, setConfirmArchive] = useState<string | null>(null);
  const [showArchive,    setShowArchive]    = useState(false);
  const [memberError,    setMemberError]    = useState('');
  const [sending,        setSending]        = useState(false);
  // 任务活动态：驱动「正在思考」气泡 + 顶部状态条；running 常驻，done/error 短暂闪现后自动清除。
  const [activity,       setActivity]       = useState<{ phase: 'running' | 'done' | 'error'; label: string } | null>(null);
  const activityTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // 实时活动流：会议室 Agent 回复时的真实思考过程，按 run_id 分流（并行步骤里多个 Agent 同时发言）。
  // text=回复正文逐字累积；actions=Agent 的工具动作（检索代码/读文件…）。done 事件撤下卡片。
  const [thinking, setThinking] = useState<Record<string, { agentId: string; agentName: string; actions: string[]; text: string }>>({});
  // 发送回执：刚发出的「我」消息短暂显示「已送达」。
  const [justSentId,     setJustSentId]     = useState('');
  const sentTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [loadError,      setLoadError]      = useState('');

  // 设置活动态；done/error 自动在 2.4s 后清除，running 常驻直到下一次状态变更。
  const flashActivity = useCallback((phase: 'running' | 'done' | 'error', label: string) => {
    if (activityTimer.current) { clearTimeout(activityTimer.current); activityTimer.current = null; }
    setActivity({ phase, label });
    // done/error 短暂闪现后清除；running 设一个较长的兜底，避免漏收 completed 事件时永久卡住。
    activityTimer.current = setTimeout(() => setActivity(null), phase === 'running' ? 240000 : 2400);
  }, []);
  const markSent = useCallback((id: string) => {
    setJustSentId(id);
    if (sentTimer.current) clearTimeout(sentTimer.current);
    sentTimer.current = setTimeout(() => setJustSentId(cur => (cur === id ? '' : cur)), 3500);
  }, []);
  useEffect(() => () => {
    if (activityTimer.current) clearTimeout(activityTimer.current);
    if (sentTimer.current) clearTimeout(sentTimer.current);
  }, []);
  // 思考气泡出现时滚到底，确保用户看到「Agent 正在思考」。
  useEffect(() => {
    if (activity?.phase === 'running') {
      setTimeout(() => { if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight; }, 60);
    }
  }, [activity?.phase]);
  const [quoteDraft,     setQuoteDraft]     = useState<QuoteDraft | null>(null);
  const [bubbleMenu,     setBubbleMenu]     = useState<BubbleMenuState | null>(null);
  const [reader,         setReader]         = useState<{ message: Message; author: string } | null>(null);
  const [readerScale,    setReaderScale]    = useState(() => Number(localStorage.getItem('conv.readerScale')) || 1.15);
  const [contextAttachments, setContextAttachments] = useState<ConversationAttachment[]>([]);
  const [windowSize,         setWindowSize]         = useState(20);
  const [listCollapsed,  setListCollapsed]  = useState(() => localStorage.getItem('conv.listCollapsed') === '1');
  const toggleList = () => setListCollapsed(v => { localStorage.setItem('conv.listCollapsed', v ? '0' : '1'); return !v; });

  const scrollRef       = useRef<HTMLDivElement>(null);
  const readerScrollRef = useRef<HTMLDivElement>(null);
  const headerActionsRef= useRef<HTMLDivElement>(null);
  const searchInputRef  = useRef<HTMLInputElement>(null);
  const messageRefs     = useRef<Record<string, HTMLDivElement | null>>({});

  // 实时活动流的 token 节流缓冲：LLM 每秒可吐数十个 token，逐个 setState 会按 token 频率
  // 重渲整条消息列表 + 重解析思考气泡的 Markdown，长会话下明显卡顿。改为把增量先攒进 ref，
  // 每 ~60ms 合并一次提交，渲染频率封顶 ~16fps，与流畅可读性兼得。tool/done 事件低频，仍即时处理。
  const thinkBuf   = useRef<Record<string, { agentId: string; agentName: string; text: string }>>({});
  const thinkTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const flushThink = useCallback(() => {
    thinkTimer.current = null;
    const buf = thinkBuf.current;
    thinkBuf.current = {};
    const rids = Object.keys(buf);
    if (rids.length === 0) return;
    setThinking(t => {
      const next = { ...t };
      for (const rid of rids) {
        const b = buf[rid];
        const cur = next[rid] || { agentId: b.agentId, agentName: b.agentName, actions: [], text: '' };
        next[rid] = { ...cur, agentName: cur.agentName || b.agentName, text: cur.text + b.text };
      }
      return next;
    });
  }, []);
  useEffect(() => () => { if (thinkTimer.current) clearTimeout(thinkTimer.current); }, []);
  // 是否贴在底部附近（用户没有上滚看历史）。仅贴底时才自动跟随，避免上滚阅读 / 点头像跳转时被拽回底部。
  const atBottomRef = useRef(true);
  // 思考卡片随流式输出 / 工具调用加载而增高时，自动跟随到底（内容向上顿动），免去手动反复下滚；
  // 用 useLayoutEffect 在绘制前贴底、无闪动；不贴底（在看历史）则不打扰。
  useLayoutEffect(() => {
    if (atBottomRef.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [thinking]);

  // Ref keeps the event listener closure up-to-date without re-registering it.
  const activeRef = useRef(active);
  activeRef.current = active;
  // 供滚动可见性计算读取最新 msgs，避免把 msgs 进 useCallback 依赖反复重建监听。
  const msgsRef = useRef(msgs);
  msgsRef.current = msgs;

  // ── Stable data-fetching callbacks ─────────────────────────────────────────

  const loadConvs = useCallback(async () => {
    const [cs, as, ps] = await Promise.all([listConversations(), listAgents(), listProjects()]);
    setConvs(cs);
    // 把内置「编码 Agent」并入列表：让所有由页面 `agents` 派生的 agentMap（消息作者名/头像、
    // @提及解析）都能查到它；其 mentionable/visible_in_chat=false 故不会混入成员选择器。
    const withBuiltin = [...as, builtinCodeAgent()];
    setAgents(withBuiltin);
    primeAgents(withBuiltin);  // 同步全局 Agent store（Markdown @提及 / Avatar 查表的真源）
    setProjects(ps);
    // Keep the current/persisted conversation if it still exists; otherwise fall back
    // to the first group conversation (groups listed before directs in the UI).
    setActive(cur => (cur && cs.some(c => c.id === cur))
      ? cur
      : (cs.find(c => c.conv_type === 'group')?.id || cs[0]?.id || ''));
  }, []);

  const loadMsgs = useCallback(async (cid: string) => {
    if (!cid) return;
    const ms = await listMessages(cid);
    setMsgs(ms);
    setTimeout(() => { if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight; }, 50);
  }, []);

  const loadContextAttachments = useCallback(async (cid: string) => {
    if (!cid) {
      setContextAttachments([]);
      return;
    }
    const attachments = await listConversationAttachments(cid);
    setContextAttachments(attachments);
  }, []);

  // ── Effects ─────────────────────────────────────────────────────────────────

  // 1. Initial data load (fires once on mount).
  useEffect(() => {
    loadConvs().catch(e => setLoadError(String(e)));
  }, [loadConvs]);

  // 2. Load messages when active conversation changes.
  //    markConversationRead is called with a 500 ms delay so it does not race
  //    with the list_messages response that unblocks the chat panel.
  useEffect(() => {
    if (!active) { setMsgs([]); setContextAttachments([]); return; }
    try { sessionStorage.setItem('AutoForge:active-conv', active); } catch { /* ignore */ }

    let alive = true;
    Promise.all([loadMsgs(active), loadContextAttachments(active)]).then(() => {
      if (!alive) return;
      setConvs(cs => cs.map(c => c.id === active ? { ...c, unread: 0 } : c));
      setLoadError('');
    }).catch(e => { if (alive) setLoadError(String(e)); });

    // 恢复在途任务指示：切换会话或重新进入会议室页时，若该会话仍有 running 的后台任务，
    // 重新点亮「正在思考」气泡——后台任务本就 detached 持续执行，这里让前端忠实反映它。
    listConversationTasks(active).then(tasks => {
      if (alive && tasks[0]?.status === 'running') flashActivity('running', '调度器安排任务中…');
    }).catch(() => {});

    const readTimer = setTimeout(() => {
      markConversationRead(active)
        .then(() => window.dispatchEvent(new Event('AutoForge:badges-refresh')))
        .catch(() => {});
    }, 500);

    return () => { alive = false; clearTimeout(readTimer); };
  }, [active, loadMsgs, loadContextAttachments, flashActivity]);

  // 3. Reset search state when switching conversations.
  useEffect(() => {
    setShowSearch(false);
    setSearchQuery('');
    setActiveSearchId(null);
    setQuoteDraft(null);
    setBubbleMenu(null);
    setEditGroup(null);
    setActivity(null);
    setThinking({});
    setCurrentAgentId(null);
    setSegmentAgents([]);
    setSegmentStartId(null);
    visKeyRef.current = '';
    atBottomRef.current = true; // 新会话默认贴底跟随
    thinkBuf.current = {};
    if (thinkTimer.current) { clearTimeout(thinkTimer.current); thinkTimer.current = null; }
    setJustSentId('');
    setWsRefs([]);
  }, [active]);

  // 4. Auto-focus search input when panel opens.
  useEffect(() => {
    if (showSearch) setTimeout(() => searchInputRef.current?.focus(), 0);
  }, [showSearch]);

  // 5. Single Tauri event listener — registered once, uses activeRef to avoid
  //    re-registration on every active change.  Re-registration creates a window
  //    where unlisten() and the next listen() race with pending IPC responses.
  //    The `cancelled` guard prevents StrictMode's double-invocation from
  //    leaking a second listener.
  useEffect(() => {
    if (!isTauri) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let timer: ReturnType<typeof setTimeout> | null = null;

    listen<unknown>('autoforge://event', e => {
      const ev = e.payload as { type?: string; conversation_id?: string; status?: string; run_id?: string; agent_id?: string; agent_name?: string; kind?: string; text?: string };
      // 实时活动流：高频思考增量，仅更新本地态、不触发消息重载，且不进 300ms 去抖。
      if (ev?.type === 'agent_thinking') {
        if (!activeRef.current || ev.conversation_id !== activeRef.current) return;
        const rid = ev.run_id || '';
        if (ev.kind === 'done') {
          delete thinkBuf.current[rid]; // 丢弃未提交的残余 token（正式消息即将落地）
          setThinking(t => { if (!(rid in t)) return t; const n = { ...t }; delete n[rid]; return n; });
          return;
        }
        if (ev.kind === 'tool') {
          // 工具动作低频，即时提交（与 token 文本是独立 UI 区，无需保序）。
          setThinking(t => {
            const cur = t[rid] || { agentId: ev.agent_id || '', agentName: ev.agent_name || '', actions: [], text: '' };
            return { ...t, [rid]: { ...cur, agentName: cur.agentName || ev.agent_name || '', actions: [...cur.actions, ev.text || ''] } };
          });
          return;
        }
        // token：攒进缓冲，按 ~60ms 节流合并提交（见 flushThink）。
        const b = thinkBuf.current[rid] || { agentId: ev.agent_id || '', agentName: ev.agent_name || '', text: '' };
        thinkBuf.current[rid] = { ...b, text: b.text + (ev.text || '') };
        if (!thinkTimer.current) thinkTimer.current = setTimeout(flushThink, 60);
        return;
      }
      if (ev?.type !== 'message_received' && ev?.type !== 'conversation_task_updated') return;
      // 活动态立即更新（不进 300ms 去抖），让顶部状态条 / 思考气泡尽快反映后端进度。
      if (ev.type === 'conversation_task_updated' && !!activeRef.current && ev.conversation_id === activeRef.current) {
        if (ev.status === 'running') flashActivity('running', '调度器安排任务中…');
        else if (ev.status === 'completed') { flashActivity('done', '已完成'); thinkBuf.current = {}; setThinking({}); }
        else if (ev.status === 'failed') { flashActivity('error', '处理失败'); thinkBuf.current = {}; setThinking({}); }
      }
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        const cur = activeRef.current;
        const isActive = ev.conversation_id === cur && !!cur;
        if (isActive) {
          setConvs(cs => cs.map(c => c.id === cur ? { ...c, unread: 0 } : c));
          loadMsgs(cur).catch(() => {});
          markConversationRead(cur)
            .then(() => {
              window.dispatchEvent(new Event('AutoForge:badges-refresh'));
              loadConvs().catch(() => {});
            })
            .catch(() => { loadConvs().catch(() => {}); });
        } else {
          loadConvs().catch(() => {});
        }
      }, 300);
    }).then(fn => {
      if (cancelled) fn(); // immediately unregister if StrictMode already cleaned up
      else unlisten = fn;
    }).catch(() => {});

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      unlisten?.();
    };
  }, [loadConvs, loadMsgs, flashActivity, flushThink]); // stable callbacks — this effect runs once

  // 6. Close panels when clicking outside the header actions area.
  useEffect(() => {
    if (!showMembers && !showContext && !showSearch && !showHeadMore) return;
    const close = (e: PointerEvent) => {
      if (!(e.target instanceof Node)) return;
      if (headerActionsRef.current?.contains(e.target)) return;
      setShowMembers(false); setShowContext(false); setShowSearch(false); setShowHeadMore(false); setMemberError('');
    };
    document.addEventListener('pointerdown', close);
    return () => document.removeEventListener('pointerdown', close);
  }, [showMembers, showContext, showSearch, showHeadMore]);

  useEffect(() => {
    if (!bubbleMenu) return;
    const close = () => setBubbleMenu(null);
    const closeOnEscape = (e: KeyboardEvent) => { if (e.key === 'Escape') close(); };
    document.addEventListener('pointerdown', close);
    document.addEventListener('keydown', closeOnEscape);
    return () => {
      document.removeEventListener('pointerdown', close);
      document.removeEventListener('keydown', closeOnEscape);
    };
  }, [bubbleMenu]);

  // Load project files + workspace files when context panel opens for a project-linked group chat
  useEffect(() => {
    if (!showContext) { setProjectFiles([]); setWorkspaceFiles([]); return; }
    const c = convs.find(c => c.id === active);
    if (!c?.project_id) { setProjectFiles([]); setWorkspaceFiles([]); return; }
    const pid = c.project_id;
    Promise.all([
      listProjectFiles(pid, active).catch(() => [] as ProjectContextFile[]),
      ensureWorkspaceDirs(pid).then(() => listWorkspaceFiles(pid)).catch(() => [] as WorkspaceFile[]),
    ]).then(([pf, wf]) => { setProjectFiles(pf); setWorkspaceFiles(wf); });
  }, [showContext, active, convs]);

  // ── Derived state ──────────────────────────────────────────────────────────

  const conv = useMemo(() => convs.find(c => c.id === active), [active, convs]);
  const agentMap = useMemo(() => Object.fromEntries(agents.map(a => [a.id, a])), [agents]);

  const convMembers = useMemo(
    () => conv
      ? conv.members
          .map(id => agentMap[id])
          .filter((a): a is Agent => !!a && a.visible_in_chat && a.enabled)
      : [],
    [conv, agentMap],
  );
  const availableAgents = useMemo(
    () => conv
      ? agents.filter(a => a.visible_in_chat && a.mentionable && a.enabled && !conv.members.includes(a.id))
      : [],
    [conv, agents],
  );

  const chatHeadSub = useMemo(() => {
    const names = convMembers.map(a => a.name).join(' · ');
    return names.length > 40 ? names.slice(0, 40) + '…' : names;
  }, [convMembers]);

  const contextBlocks = useMemo(() => {
    if (!showContext) return [];
    return msgs.flatMap(m => {
      try {
        const bs: BlockType[] = JSON.parse(m.content_json);
        return bs
          .filter(b => b.t === 'file' || b.t === 'image' || b.t === 'artifact' || b.t === 'code')
          .map(block => ({ block, messageId: m.id }));
      } catch { return []; }
    });
  }, [msgs, showContext]);

  const operator = useOperator();
  const normalizedQ = searchQuery.trim().toLowerCase();
  const visibleMsgCount = useMemo(() => msgs.filter(m => !m.id.startsWith('typing-')).length, [msgs]);
  const searchResults = useMemo(() => {
    if (!showSearch || !normalizedQ) return [];
    return msgs
      .filter(m => !m.id.startsWith('typing-') && msgText(m).toLowerCase().includes(normalizedQ))
      .map(m => ({ message: m, text: msgText(m).replace(/\s+/g, ' ').trim(), sender: m.from_agent ? (agentMap[m.from_agent]?.name ?? 'Agent') : operator.display_name }));
  }, [msgs, showSearch, normalizedQ, agentMap, operator.display_name]);

  // ── Actions ────────────────────────────────────────────────────────────────

  const jumpToMessage = (id: string) => {
    setActiveSearchId(id);
    messageRefs.current[id]?.scrollIntoView({ block: 'center', behavior: 'smooth' });
  };

  // 把某条消息对齐到消息区顶端并短暂高亮（堆叠头像 / 「我」按钮点击复用）。
  const jumpToMsgStart = (tid: string) => {
    setActiveSearchId(tid);
    messageRefs.current[tid]?.scrollIntoView({ block: 'start', behavior: 'smooth' });
    setTimeout(() => setActiveSearchId(cur => (cur === tid ? null : cur)), 1800);
  };
  // 左侧「轮次刻度尺 + 悬浮目录」(RoundAvatarStack) 所需数据：所有轮次(我的发言)及其预览文本，
  // 每轮挂上该轮所有 agent 回复(子列表)。单遍扫描 msgs，以「我的发言」为界切轮；文本在此算好，组件不感知 msgs/msgText。
  const stackRounds: RoundNav[] = [];
  for (const m of msgs) {
    if (m.id.startsWith('typing-')) continue;       // 跳过流式占位
    if (!m.from_agent) {
      stackRounds.push({ id: m.id, text: msgText(m).trim(), replies: [] });
    } else if (stackRounds.length > 0) {            // agent 回复挂到当前轮
      const ag = agentMap[m.from_agent];
      if (ag) stackRounds[stackRounds.length - 1].replies.push({ msgId: m.id, agent: ag, text: msgText(m).trim() });
    }
  }
  const curRoundIdx = segmentStartId ? stackRounds.findIndex(r => r.id === segmentStartId) : -1;

  // 计算当前视口内可见发言的 agent 集合（供堆叠去重）。按消息 DOM 顺序遍历，越过视口下沿即停。
  // 用 getBoundingClientRect 对照滚动容器视口，rAF 节流，仅在滚动/消息变化时触发。
  const visKeyRef = useRef('');
  const visRafRef = useRef<number | null>(null);
  const recomputeVisibleAgents = useCallback(() => {
    visRafRef.current = null;
    const sc = scrollRef.current;
    if (!sc) return;
    // 顺带更新「是否贴底」：滚动回调驱动，供流式自动跟随判定（上滚看历史时不跟随）。
    atBottomRef.current = sc.scrollHeight - sc.scrollTop - sc.clientHeight < 120;
    const scRect = sc.getBoundingClientRect();
    const msgs = msgsRef.current;
    let current: string | null = null;  // 跨越视口顶边的那条消息 = 当前正在看的气泡
    let currentIdx = -1;
    let firstVisIdx = -1;
    for (let i = 0; i < msgs.length; i++) {
      const m = msgs[i];
      const el = messageRefs.current[m.id];
      if (!el) continue;
      const r = el.getBoundingClientRect();
      if (r.top > scRect.bottom) break;     // 之后的消息都在视口下方
      if (r.bottom < scRect.top) continue;  // 整行都在视口上方
      if (firstVisIdx < 0) firstVisIdx = i; // 第一条与视口相交的消息（兜底「当前」）
      // 跨越视口顶边（行顶在视口顶之上、行底在视口顶之下）= 当前正在阅读的气泡。
      if (r.top <= scRect.top + 1 && r.bottom > scRect.top) { current = m.from_agent || null; currentIdx = i; }
    }
    if (currentIdx < 0 && firstVisIdx >= 0) { currentIdx = firstVisIdx; current = msgs[firstVisIdx].from_agent || null; }
    // 以「我的发言」(from_agent 为空) 为界，求当前位置所在那一段里发过言的 agent 集合（堆叠作用域）。
    const seg = new Set<string>();
    let segStartId: string | null = null;
    if (currentIdx >= 0) {
      let start = currentIdx;
      while (start > 0 && msgs[start].from_agent) start--; // 回退到本段起始的「我的发言」或开头
      if (msgs[start] && !msgs[start].from_agent) segStartId = msgs[start].id; // 本段起始「我的发言」
      for (let i = start; i < msgs.length; i++) {
        if (i > start && !msgs[i].from_agent) break;       // 下一条「我的发言」= 段结束
        if (msgs[i].from_agent) seg.add(msgs[i].from_agent!);
      }
    }
    const segArr = [...seg]; // Set 保留插入顺序 = 发言先后顺序
    // key 用有序 join（非排序），以便「成员相同但顺序变化」也能触发更新。
    const key = segArr.join(',') + '|' + (current ?? '') + '|' + (segStartId ?? '');
    if (key !== visKeyRef.current) {
      visKeyRef.current = key;
      setCurrentAgentId(current);
      setSegmentAgents(segArr);
      setSegmentStartId(segStartId);
    }
  }, []);
  const scheduleVisible = useCallback(() => {
    if (visRafRef.current == null) visRafRef.current = requestAnimationFrame(recomputeVisibleAgents);
  }, [recomputeVisibleAgents]);
  // 消息变化（加载/新消息/滚到底）后重算；卸载时取消挂起的 rAF。
  useEffect(() => {
    const t = setTimeout(scheduleVisible, 80);
    return () => clearTimeout(t);
  }, [msgs, scheduleVisible]);
  useEffect(() => () => { if (visRafRef.current != null) cancelAnimationFrame(visRafRef.current); }, []);

  const openBubbleMenu = (e: React.MouseEvent, message: Message, author: string) => {
    e.preventDefault();
    setBubbleMenu({
      x: Math.min(e.clientX, window.innerWidth - 148),
      y: Math.min(e.clientY, window.innerHeight - 232),
      message,
      author,
    });
  };

  const copyBubbleMessage = async () => {
    if (!bubbleMenu) return;
    try {
      await copyText(msgText(bubbleMenu.message).trim());
      setLoadError('');
    } catch (e) {
      setLoadError(String(e));
    } finally {
      setBubbleMenu(null);
    }
  };

  const quoteBubbleMessage = () => {
    if (!bubbleMenu) return;
    const text = msgText(bubbleMenu.message).replace(/\s+/g, ' ').trim();
    setQuoteDraft({
      message_id: bubbleMenu.message.id,
      author: bubbleMenu.author,
      text: text || '消息内容为空',
      created_at: bubbleMenu.message.created_at,
    });
    setBubbleMenu(null);
  };

  // 会话即入口：把一条消息「沉淀为需求」，直接走 flow 模式自动分析，
  // 与「需求草稿 card → 确认需求」一致——立即入库并进入分析 → 需求审核，
  // 而非落入待整理池（triage）需人工再整理。
  const distillBubbleToIssue = async () => {
    if (!bubbleMenu || !conv?.project_id) return;
    const text = msgText(bubbleMenu.message).trim();
    setBubbleMenu(null);
    if (!text) { flashActivity('error', '这条消息没有可沉淀的文本'); return; }
    try {
      const first = text.split(/[\n。.!?！？]/)[0]?.trim() ?? text;
      await submitIssue({
        project_id: conv.project_id,
        title: first.length > 30 ? first.slice(0, 30) : first,
        description: text,
        source_type: 'conversation',
        mode: 'flow',
      });
      setLoadError('');
      // 成功无任何提示会让用户以为「没反应」：沉淀走 flow 模式，需求立即开始分析，
      // 不会在当前会议室出现，必须显式回馈一次，并指引去哪里看。
      // 需求审核 / 全量需求总账都在「功能审计」页（Audit.tsx），不是「交付流水线」。
      flashActivity('done', '已沉淀为需求，开始分析（功能审计 → 需求审核）');
    } catch (e) { setLoadError(String(e)); }
  };

  const toggleContextBubbleMessage = async () => {
    if (!bubbleMenu) return;
    const id = bubbleMenu.message.id;
    setBubbleMenu(null);
    try {
      const updated = await toggleMessageContext(id);
      setMsgs(ms => ms.map(m => m.id === id ? { ...m, excluded_from_context: updated.excluded_from_context } : m));
    } catch (e) {
      setLoadError(String(e));
    }
  };

  // 软删除单条消息：从当前气泡列表移除，后台仅打 deleted_at 标记（保留数据可审计）。
  const deleteBubbleMessage = async () => {
    if (!bubbleMenu) return;
    const id = bubbleMenu.message.id;
    setBubbleMenu(null);
    try {
      await softDeleteMessage(id);
      setMsgs(ms => ms.filter(m => m.id !== id));
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
    } catch (e) {
      setLoadError(String(e));
    }
  };

  const openReader = () => {
    if (!bubbleMenu) return;
    setReader({ message: bubbleMenu.message, author: bubbleMenu.author });
    setBubbleMenu(null);
  };
  const bumpReaderScale = (delta: number) => {
    setReaderScale(s => {
      const next = Math.min(2, Math.max(0.85, Math.round((s + delta) * 100) / 100));
      localStorage.setItem('conv.readerScale', String(next));
      return next;
    });
  };
  useEffect(() => {
    if (!reader) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setReader(null); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [reader]);

  const onSend = async (
    text: string,
    attachments: PendingAttachment[],
    contextRefs: ConversationAttachment[],
    mentionedAgentIds: string[],
  ) => {
    if (!conv || sending) return false;
    const stagedWsRefs = wsRefs;
    setSending(true);
    setLoadError('');
    try {
      const blocks: BlockType[] = [];
      if (text) blocks.push({ t: 'md', md: text });

      for (const item of attachments) {
        const data_base64 = await fileToBase64(item.file);
        const attachment = await importAttachment({
          conversation_id: conv.id,
          file_name: item.file.name,
          mime_hint: item.file.type || '',
          data_base64,
        });
        if (item.mode === 'image' && attachment.kind !== 'image') {
          throw new Error(`${item.file.name} 不是受支持的图片`);
        }
        blocks.push(attachmentBlock(attachment));
      }

      for (const attachment of contextRefs) {
        blocks.push(contextAttachmentBlock(attachment));
      }

      // 工作区文件引用：随消息携带 .autoforge/ 相对路径，后端构建提示时按需读取内容。
      for (const ref of stagedWsRefs) {
        blocks.push(workspaceRefBlock(ref));
      }

      if (quoteDraft) {
        blocks.push({
          t: 'quote_ref',
          message_id: quoteDraft.message_id,
          author: quoteDraft.author,
          text: quoteDraft.text,
          created_at: quoteDraft.created_at,
        });
      }

      if (blocks.length === 0) return false;
      const m = await sendMessage({ conversation_id: conv.id, content_json: JSON.stringify(blocks) });
      setMsgs(ms => [...ms, m]);
      markSent(m.id);
      setQuoteDraft(null);
      setWsRefs([]);
      loadConvs().catch(() => {});
      if (attachments.length > 0) loadContextAttachments(conv.id).catch(() => {});
      setTimeout(() => { if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight; }, 50);

      // Innate 斜杠命令：不触发 AI 编排任务，改为运行知识库命令并回插系统消息。
      const slash = parseSlashCommand(text);
      if (slash) {
        const reply = await runConversationCommand({
          conversation_id: conv.id,
          command: (slash.name ?? slash.raw) as ConvCommandName,
          arg: slash.arg,
        });
        setMsgs(ms => [...ms, reply]);
        loadConvs().catch(() => {});
        setTimeout(() => { if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight; }, 50);
        return true;
      }

      const shouldStartTask = text.trim().length > 0 || contextRefs.length > 0 || attachments.length > 0 || stagedWsRefs.length > 0;
      if (shouldStartTask) {
        const directAgentIds = conv.conv_type === 'direct'
          ? conv.members.filter(id => {
              const a = agentMap[id];
              return !!a && a.enabled;
            }).slice(0, 1)
          : [];
        // 乐观置为「思考中」：让用户立刻知道 Agent 已收到并开始处理，无需等后端 running 事件。
        flashActivity('running', '调度器安排任务中…');
        await startConversationTask({
          conversation_id: conv.id,
          trigger_message_id: m.id,
          instruction: text.trim() || '请基于刚刚发送的附件和上下文回复。',
          mentioned_agent_ids: mentionedAgentIds.length > 0 ? mentionedAgentIds : directAgentIds,
          window_size: windowSize,
        });
      }
      return true;
    } catch (e) {
      setLoadError(String(e));
      flashActivity('error', '发送失败');
      return false;
    } finally {
      setSending(false);
    }
  };

  // 总结/结论快捷指令：调用后端在生成摘要的同时压缩上下文，随后重载消息（事件也会触发重载）。
  // 返回是否成功，供 Composer 决定快捷 tag 显示「已完成」还是回退。
  const onCompress = async (mode: 'summary' | 'conclusion'): Promise<boolean> => {
    if (!conv || sending) return false;
    setSending(true);
    setLoadError('');
    flashActivity('running', mode === 'conclusion' ? '正在收敛结论…' : '正在总结上下文…');
    try {
      await compressConversationContext({ conversation_id: conv.id, mode });
      await loadMsgs(conv.id);
      loadConvs().catch(() => {});
      flashActivity('done', mode === 'conclusion' ? '结论已生成' : '上下文已压缩');
      setTimeout(() => { if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight; }, 50);
      return true;
    } catch (e) {
      setLoadError(String(e));
      flashActivity('error', '处理失败');
      return false;
    } finally {
      setSending(false);
    }
  };

  const handleNewGroup = async (name: string, memberIds: string[], projectId: string | null) => {
    const c = await createGroupConversation(name, memberIds, undefined, undefined, projectId);
    await loadConvs();
    setActive(c.id);
    setShowNew(false);
  };

  const saveGroupInfo = async (conversationId: string, name: string, projectId: string | null) => {
    setLoadError('');
    try {
      const updated = await updateGroupConversation(conversationId, name, projectId);
      setConvs(cs => cs.map(c => c.id === updated.id ? updated : c));
      setEditGroup(null);
      setShowContext(false);
      setWsRefs([]);
      if (active === updated.id) {
        loadContextAttachments(updated.id).catch(() => {});
      }
    } catch (e) {
      setLoadError(String(e));
    }
  };

  const addMember = async (agentId: string) => {
    if (!conv) return;
    setMemberError('');
    try {
      const updated = await addConversationMember(conv.id, agentId);
      setConvs(cs => cs.map(c => c.id === updated.id ? updated : c));
    } catch (e) { setMemberError(String(e)); }
  };

  const removeMember = async (agentId: string) => {
    if (!conv) return;
    setMemberError('');
    try {
      const updated = await removeConversationMember(conv.id, agentId);
      setConvs(cs => cs.map(c => c.id === updated.id ? updated : c));
    } catch (e) { setMemberError(String(e)); }
  };

  const dissolveGroup = async () => {
    if (!confirmDissolve) return;
    const id = confirmDissolve;
    setMemberError('');
    try {
      await deleteGroupConversation(id);
      const remaining = convs.filter(c => c.id !== id);
      setConvs(remaining);
      if (active === id) { setActive(remaining[0]?.id ?? ''); setMsgs([]); }
      setShowMembers(false);
      setConfirmDissolve(null);
    } catch (e) {
      setMemberError(String(e));
      setConfirmDissolve(null);
      setShowMembers(true);
    }
  };

  const archiveAndClearConversation = async () => {
    if (!confirmArchive) return;
    const id = confirmArchive;
    setLoadError('');
    try {
      await archiveConversation(id);
      const remaining = await listConversations();
      setConvs(remaining);
      setMsgs([]);
      setContextAttachments([]);
      setQuoteDraft(null);
      setBubbleMenu(null);
      setConfirmArchive(null);
      window.dispatchEvent(new Event('AutoForge:badges-refresh'));
      if (!remaining.some(c => c.id === id)) {
        setActive(remaining[0]?.id ?? '');
      }
    } catch (e) {
      setLoadError(String(e));
      setConfirmArchive(null);
    }
  };

  // ── Render ─────────────────────────────────────────────────────────────────

  return (
    <>
      <ConvList convs={convs} agents={agents} active={active} onSelect={setActive} onNew={() => setShowNew(true)} onOpenArchive={() => setShowArchive(true)} collapsed={listCollapsed} onToggleCollapse={toggleList} />

      {conv ? (
        <div className="content" style={{ position: 'relative' }}>
          {/* 左侧「对话轮次刻度尺 + 悬浮目录」(独立组件)：默认只见极简刻度尺；
              hover 弹出目录——标题=我的发言，子列表=该轮所有 agent 回复(hover 标题展开)，点击跳转。 */}
          {conv.conv_type === 'group' && (
            <RoundAvatarStack
              key={conv.id}
              rounds={stackRounds}
              currentRoundIndex={curRoundIdx}
              onJump={jumpToMsgStart}
            />
          )}
          {/* ── Chat header ── */}
          <div className="chat-head">
            {listCollapsed && (
              <button className="icon-btn" title="展开对话列表" onClick={toggleList} style={{ marginRight: 2 }}>
                <Icon name="columns" size={18} />
              </button>
            )}
            {conv.conv_type === 'group'
              ? <div className="av sq" style={{ width: 38, height: 38, background: conv.color, fontSize: 'var(--text-title)' }}>{conv.initial ?? conv.name?.[0] ?? '群'}</div>
              : (() => { const a = agentMap[conv.members[0]]; return a ? <Avatar agent={a} size={38} status={conv.unread > 0 ? 'online' : undefined} /> : null; })()}
            <div className="chat-head-info">
              <div className="chat-head-title">
                {conv.conv_type === 'group' ? conv.name : agentMap[conv.members[0]]?.name ?? 'Agent'}
                {conv.conv_type === 'group' && <span className="chip" style={{ padding: '1px 7px', fontSize: 'var(--text-micro)' }}>{conv.members.length} 成员</span>}
                {conv.conv_type === 'group' && conv.project_id && (
                  <span className="chip" style={{ padding: '1px 7px', fontSize: 'var(--text-micro)', background: 'var(--ember-tint)', color: 'var(--ember)', display: 'inline-flex', alignItems: 'center', gap: 3 }}>
                    <Icon name="folder" size={9} />
                    {projects.find(p => p.id === conv.project_id)?.name ?? '项目'}
                  </span>
                )}
              </div>
              <div className="chat-head-sub">{chatHeadSub}</div>
            </div>
            <div className="chat-head-actions" ref={headerActionsRef} style={{ position: 'relative' }}>
              {conv.conv_type === 'group' && (
                <button className="member-stack" title="群成员列表" onClick={() => { setShowMembers(v => !v); setShowContext(false); setShowSearch(false); setShowHeadMore(false); }}
                  style={{ background: 'transparent', border: 0, padding: '0 4px', cursor: 'pointer' }}>
                  {convMembers.slice(0, 4).map(a => <Avatar key={a.id} agent={a} size={24} />)}
                </button>
              )}
              {conv.conv_type === 'group' && <div className="chat-head-sep" />}
              <button className={`icon-btn${showContext ? ' on' : ''}`} title="会议室上下文与附件" onClick={() => { setShowContext(v => !v); setShowMembers(false); setShowSearch(false); setShowHeadMore(false); }}>
                <Icon name="layers" size={18} />
              </button>
              <button className={`icon-btn${showSearch ? ' on' : ''}`} title="搜索会议室" onClick={() => { setShowSearch(v => !v); setShowMembers(false); setShowContext(false); setShowHeadMore(false); }}>
                <Icon name="search" size={18} />
              </button>
              <button className={`icon-btn${showHeadMore ? ' on' : ''}`} title="更多操作" onClick={() => { setShowHeadMore(v => !v); setShowMembers(false); setShowContext(false); setShowSearch(false); }}>
                <Icon name="dots" size={18} />
              </button>

              {/* More-actions menu */}
              {showHeadMore && (
                <div className="mention-pop" style={{ right: 0, left: 'auto', top: 38, bottom: 'auto', width: 220 }}>
                  <div
                    className="mention-row"
                    onClick={async () => { await copyText(conv.id); setIdCopied(true); setTimeout(() => setIdCopied(false), 1400); }}
                  >
                    <Icon name={idCopied ? 'check' : 'copy'} size={15} style={{ color: idCopied ? 'var(--green)' : 'var(--text-3)' }} />
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div className="nm">{idCopied ? '已复制编号' : '复制会议室编号'}</div>
                      <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-faint)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{conv.id}</div>
                    </div>
                  </div>
                  {conv.conv_type === 'group' && (
                    <div className="mention-row" onClick={() => { setEditGroup(conv); setShowHeadMore(false); }}>
                      <Icon name="edit" size={15} style={{ color: 'var(--text-3)' }} />
                      <div style={{ flex: 1, minWidth: 0 }}><div className="nm">编辑会议室</div></div>
                    </div>
                  )}
                  <div className="mention-row" onClick={() => { setShowArchive(true); setShowHeadMore(false); }}>
                    <Icon name="inbox" size={15} style={{ color: 'var(--text-3)' }} />
                    <div style={{ flex: 1, minWidth: 0 }}><div className="nm">归档区 · 检索回顾</div></div>
                  </div>
                  <div
                    className="mention-row"
                    style={visibleMsgCount === 0 ? { opacity: 0.45, pointerEvents: 'none' } : undefined}
                    onClick={() => { if (visibleMsgCount === 0) return; setConfirmArchive(conv.id); setShowHeadMore(false); }}
                  >
                    <Icon name="package" size={15} style={{ color: 'var(--ember)' }} />
                    <div style={{ flex: 1, minWidth: 0 }}><div className="nm">归档会议室内容</div></div>
                  </div>
                </div>
              )}

              {/* Members panel */}
              {conv.conv_type === 'group' && showMembers && (
                <div className="mention-pop" style={{ right: 0, left: 'auto', top: 38, bottom: 'auto', width: 280 }}>
                  <div className="mention-pop-label">群成员</div>
                  {memberError && <div style={{ padding: '6px 8px', color: 'var(--red)', fontSize: 'var(--text-label)' }}>{memberError}</div>}
                  {convMembers.map(a => (
                    <div key={a.id} className="mention-row">
                      <Avatar agent={a} size={32} />
                      <div style={{ minWidth: 0, flex: 1 }}>
                        <div className="nm">{a.name}</div>
                        <div className="rl">{a.role || a.name_en}</div>
                      </div>
                      {convMembers.length > 2 && (
                        <button className="icon-btn" title="移除成员" style={{ width: 26, height: 26 }}
                          onClick={e => { e.stopPropagation(); removeMember(a.id); }}>
                          <Icon name="x" size={13} />
                        </button>
                      )}
                    </div>
                  ))}
                  {convMembers.length === 0 && <div className="empty-compact" style={{ padding: '10px 8px' }}>暂无成员信息</div>}
                  <div className="mention-pop-label" style={{ paddingTop: 10 }}>快速添加</div>
                  {availableAgents.map(a => (
                    <div key={a.id} className="mention-row" onClick={() => addMember(a.id)}>
                      <Avatar agent={a} size={30} />
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div className="nm">{a.name}</div>
                        <div className="rl">{a.role || a.name_en}</div>
                      </div>
                      <Icon name="plus" size={14} style={{ color: 'var(--ember)' }} />
                    </div>
                  ))}
                  {availableAgents.length === 0 && <div className="empty-compact" style={{ padding: 8 }}>所有 Agent 均已在群内</div>}
                  <div style={{ height: 1, background: 'var(--border)', margin: '6px 4px' }} />
                  <button className="btn btn-danger" style={{ width: '100%', justifyContent: 'center' }} onClick={() => setConfirmDissolve(conv.id)}>
                    <Icon name="trash" size={14} />解散群聊
                  </button>
                </div>
              )}

              {/* Context panel */}
              {showContext && (
                <div className="mention-pop" style={{ right: 0, left: 'auto', top: 38, bottom: 'auto', width: 360, maxHeight: 560, overflowY: 'auto' }}>
                  <div className="mention-pop-label">会议室上下文与附件</div>
                  <div style={{ padding: '7px 8px 4px' }}>
                    <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-3)', lineHeight: 'var(--leading-relaxed)', marginBottom: 8 }}>
                      共 {visibleMsgCount} 条消息 ·
                      已排除 {msgs.filter(m => !m.id.startsWith('typing-') && m.excluded_from_context).length} 条 ·
                      上下文块 {contextBlocks.length} 个
                    </div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 'var(--text-label)' }}>
                      <span style={{ color: 'var(--text-3)', whiteSpace: 'nowrap' }}>窗口大小</span>
                      <input
                        type="range" min={5} max={50} step={5} value={windowSize}
                        onChange={e => setWindowSize(Number(e.target.value))}
                        style={{ flex: 1, accentColor: 'var(--ember)' }}
                      />
                      <span style={{ color: 'var(--ember)', fontFamily: 'var(--font-mono)', minWidth: 28, textAlign: 'right' }}>{windowSize}</span>
                    </div>
                    <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', marginTop: 3 }}>
                      发送时取最近 {windowSize} 条（排除标记的消息）
                    </div>
                  </div>
                  <div style={{ height: 1, background: 'var(--border)', margin: '4px 0' }} />
                  {contextBlocks.slice(-8).reverse().map(({ block: b, messageId }, i) => (
                    <div
                      key={`${messageId}-${i}`}
                      className="mention-row"
                      title="定位到这条会议室内容"
                      onClick={() => {
                        jumpToMessage(messageId);
                        setShowContext(false);
                      }}
                    >
                      <div className="cfg-logo" style={{ width: 30, height: 30, background: b.t === 'image' ? 'var(--blue)' : b.t === 'file' ? 'var(--amber)' : 'var(--ember)' }}>
                        <Icon name={b.t === 'image' ? 'image' : b.t === 'file' ? 'file' : b.t === 'code' ? 'code' : 'zap'} size={15} />
                      </div>
                      <div style={{ minWidth: 0, flex: 1 }}>
                        <div className="nm">{b.t === 'file' ? b.name : b.t === 'image' ? b.label : b.t === 'code' ? `${b.lang} 代码片段` : b.title}</div>
                        <div className="rl">{b.t === 'file' || b.t === 'image' ? b.meta : b.t === 'artifact' ? b.kind : '可引用上下文'}</div>
                      </div>
                      {(b.t === 'file' || b.t === 'image') && b.id && (
                        <button
                          className="icon-btn"
                          title="用本机默认程序打开"
                          style={{ width: 26, height: 26, flex: 'none' }}
                          onClick={e => {
                            e.stopPropagation();
                            openAttachment(b.id!).catch(err => setLoadError(String(err)));
                          }}
                        >
                          <Icon name="external" size={13} />
                        </button>
                      )}
                    </div>
                  ))}
                  {contextBlocks.length === 0 && !conv?.project_id && (
                    <div className="empty-compact" style={{ padding: '10px 8px' }}>暂无附件或上下文块</div>
                  )}

                  {/* Workspace (.autoforge) section */}
                  {conv?.project_id && (
                    <>
                      <div style={{ height: 1, background: 'var(--border)', margin: '6px 0 2px' }} />
                      <div className="mention-pop-label" style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
                        <Icon name="folder" size={12} style={{ color: 'var(--ember)' }} />
                        工作区文件
                        <div style={{ marginLeft: 'auto', display: 'flex', gap: 4 }}>
                          {(['docs', 'specs', 'deliverables'] as const).map(tab => (
                            <button key={tab} onClick={() => setWorkspaceTab(tab)}
                              className="btn btn-sm"
                              style={{ padding: '1px 8px', fontSize: 'var(--text-micro)', background: workspaceTab === tab ? 'var(--ember)' : undefined, color: workspaceTab === tab ? '#fff' : undefined }}>
                              {tab}
                            </button>
                          ))}
                        </div>
                      </div>
                      {/* File list：点击即把文件作为附件引用暂存到输入框上方，发送时随消息携带 */}
                      {workspaceFiles.filter(f => f.subfolder === workspaceTab).length === 0 ? (
                        <div className="empty-compact" style={{ padding: '8px 8px' }}>
                          .autoforge/{workspaceTab}/ 暂无文件
                          <span style={{ display: 'block', fontSize: 'var(--text-micro)', color: 'var(--text-faint)', marginTop: 3 }}>
                            让 Agent 创建文档，或直接点击 Artifact 的"存入 {workspaceTab}"按钮
                          </span>
                        </div>
                      ) : (
                        workspaceFiles.filter(f => f.subfolder === workspaceTab).map(f => {
                          const referenced = wsRefs.some(r => r.path === f.rel_path);
                          return (
                          <div key={f.rel_path} className="mention-row" style={{ cursor: 'pointer' }}
                            title={referenced ? '点击取消引用' : '点击引用到输入框'}
                            onClick={() => setWsRefs(refs => referenced
                              ? refs.filter(r => r.path !== f.rel_path)
                              : [...refs, { path: f.rel_path, name: f.name }])}>
                            <div className="cfg-logo" style={{ width: 28, height: 28, background: 'var(--ember)', flexShrink: 0 }}>
                              <Icon name="file" size={13} />
                            </div>
                            <div style={{ minWidth: 0, flex: 1 }}>
                              <div className="nm">{f.name}</div>
                              <div className="rl" style={{ fontSize: 'var(--text-caption)' }}>{f.modified_at} · {(f.size_bytes / 1024).toFixed(1)} KB</div>
                            </div>
                            <Icon name={referenced ? 'check' : 'plus'} size={13}
                              style={{ color: referenced ? 'var(--ember)' : 'var(--text-3)', flexShrink: 0 }} />
                          </div>
                        ); })
                      )}
                      <div style={{ height: 1, background: 'var(--border)', margin: '4px 0 2px' }} />

                      {/* Project context files (claude.md etc.) */}
                      <div className="mention-pop-label" style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
                        <Icon name="zap" size={12} style={{ color: 'var(--ember)' }} />
                        只读上下文文件
                      </div>
                      {projectFiles.length === 0 && (
                        <div className="empty-compact" style={{ padding: '6px 8px' }}>项目目录无可引用文件</div>
                      )}
                      {projectFiles.slice(0, 12).map(f => (
                        <div key={f.rel_path} className="mention-row" style={{ paddingTop: 5, paddingBottom: 5 }}>
                          <div className="cfg-logo" style={{ width: 26, height: 26, flexShrink: 0,
                            background: f.is_priority ? 'var(--ember)' : f.pinned ? 'var(--amber)' : 'var(--bg-4)' }}>
                            <Icon name={f.is_priority ? 'zap' : 'file'} size={12} />
                          </div>
                          <div style={{ minWidth: 0, flex: 1 }}>
                            <div className="nm" style={{ fontSize: 'var(--text-control)', display: 'flex', alignItems: 'center', gap: 4 }}>
                              {f.name}
                              {f.is_priority && (
                                <span style={{ fontSize: 'var(--text-micro)', background: 'var(--ember-tint)', color: 'var(--ember)', borderRadius: 4, padding: '0 4px' }}>自动注入</span>
                              )}
                            </div>
                            <div className="rl" style={{ fontSize: 'var(--text-micro)' }}>{f.rel_path}</div>
                          </div>
                          {!f.is_priority && (
                            <button
                              className="icon-btn"
                              title={f.pinned ? '从只读上下文移除' : '加入只读上下文'}
                              style={{ width: 24, height: 24, flex: 'none', color: f.pinned ? 'var(--ember)' : undefined }}
                              onClick={e => {
                                e.stopPropagation();
                                const op = f.pinned
                                  ? removeConversationProjectContext(active, f.rel_path)
                                  : addConversationProjectContext(active, f.rel_path);
                                op.then(() => {
                                  if (conv?.project_id) {
                                    listProjectFiles(conv.project_id, active).then(setProjectFiles).catch(() => {});
                                  }
                                }).catch(err => setLoadError(String(err)));
                              }}
                            >
                              <Icon name={f.pinned ? 'check' : 'plus'} size={12} />
                            </button>
                          )}
                        </div>
                      ))}
                      <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-faint)', padding: '4px 8px 8px' }}>
                        只读文件仅注入上下文供参考，可写范围仅限 .autoforge/docs/、.autoforge/specs/ 和 .autoforge/deliverables/
                      </div>
                    </>
                  )}
                </div>
              )}

              {/* Search panel */}
              {showSearch && (
                <div className="mention-pop chat-search-pop" style={{ right: 0, left: 'auto', top: 38, bottom: 'auto', width: 360 }}>
                  <div className="mention-pop-label">搜索会议室记录</div>
                  <div className="chat-search-box">
                    <Icon name="search" size={15} />
                    <input ref={searchInputRef} value={searchQuery} onChange={e => setSearchQuery(e.target.value)} placeholder="输入关键词搜索当前会议室" />
                  </div>
                  <div className="chat-search-meta">
                    {normalizedQ ? `找到 ${searchResults.length} 条匹配消息` : `当前会议室 ${visibleMsgCount} 条消息`}
                  </div>
                  <div className="chat-search-results scroll">
                    {normalizedQ && searchResults.map(({ message, text, sender }) => (
                      <div key={message.id} className={'mention-row chat-search-row' + (activeSearchId === message.id ? ' mention-active' : '')} onClick={() => jumpToMessage(message.id)}>
                        <div style={{ minWidth: 0, flex: 1 }}>
                          <div className="nm">{sender}</div>
                          <div className="rl">{text || '消息内容为空'}</div>
                        </div>
                        <span className="chat-search-time" title={fmtFull(message.created_at)}>
                          {fmtMsgTime(message.created_at)}
                        </span>
                      </div>
                    ))}
                    {normalizedQ && searchResults.length === 0 && (
                      <div className="empty-compact" style={{ padding: '12px 8px' }}>没有匹配的消息</div>
                    )}
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* ── Messages ── */}
          <div className="msgs scroll" ref={scrollRef} onScroll={scheduleVisible}>
            {msgs.map((m, i) => (
              <MessageRow
                key={m.id ?? i}
                m={m}
                agents={agents}
                isGroup={conv.conv_type === 'group'}
                highlighted={activeSearchId === m.id}
                searchTerm={showSearch ? searchQuery.trim() : ''}
                rowRef={el => { messageRefs.current[m.id] = el; }}
                onBubbleContextMenu={openBubbleMenu}
                projectId={conv.project_id ?? undefined}
                receipt={m.id === justSentId}
              />
            ))}
            {/* 实时活动流：每个正在发言的 Agent 一张卡片，显示其工具动作 + 逐字流入的回复正文 */}
            {Object.entries(thinking).map(([rid, t]) => {
              const ta = agentMap[t.agentId];
              const accent = ta?.color || 'var(--ember)';
              return (
              <div key={rid} className="msg rise">
                {/* 工作中头像：bot 图标 + 向外扩散的 ember 脉冲环，传达「正在锻造」的活感 */}
                <div style={{ position: 'relative', width: 36, height: 36, flex: 'none' }}>
                  <div className="av" style={{ width: 36, height: 36, background: 'var(--ember-tint-strong)', color: 'var(--ember-soft)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                    <Icon name="bot" size={20} />
                  </div>
                  <span style={{ position: 'absolute', inset: 0, borderRadius: 13, border: '1.5px solid var(--ember)', animation: 'pulse-ring 1.9s ease-out infinite', pointerEvents: 'none' }} />
                </div>
                <div className="msg-body">
                  <div className="typing-cap" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                    <span style={{ width: 6, height: 6, borderRadius: 99, background: 'var(--ember)', animation: 'pulse 1.4s ease-in-out infinite', flex: 'none' }} />
                    <span style={{ color: accent, fontWeight: 600 }}>{t.agentName || 'Agent'}</span>
                    <span>正在工作…</span>
                  </div>
                  {(() => {
                    // 工具动作 + 流式正文拍平成行，只取最后三行；包进带 ember 左轴 + 顶部流光的「控制台」面板，
                    // 最新一行更亮，营造正在处理的活感（最新内容始终在底部可见）。
                    const lines = [
                      ...t.actions.map(s => ({ mono: true, s })),
                      ...(t.text ? t.text.split('\n').map(s => ({ mono: false, s })) : []),
                    ].filter(l => l.s.trim());
                    const tail = lines.slice(-3);
                    if (tail.length === 0) return <div className="bubble typing-bubble"><div className="typing"><i /><i /><i /></div></div>;
                    return (
                      <div style={{
                        position: 'relative', overflow: 'hidden', maxWidth: '90%',
                        display: 'flex', flexDirection: 'column', gap: 2, lineHeight: 'var(--leading-normal)',
                        background: 'var(--bg-2)', borderLeft: '2px solid var(--ember)',
                        borderRadius: '4px 10px 10px 4px', padding: '7px 12px', boxShadow: 'var(--shadow-sm)',
                      }}>
                        {/* 顶部 ember 流光：表示正在处理 */}
                        <span style={{ position: 'absolute', top: 0, left: 0, right: 0, height: 2, overflow: 'hidden' }}>
                          <span style={{ position: 'absolute', inset: 0, background: 'linear-gradient(90deg, transparent, var(--ember), transparent)', transform: 'translateX(-100%)', animation: 'skel-sweep 1.5s ease-in-out infinite' }} />
                        </span>
                        {tail.map((l, i) => (
                          <div key={i} style={{
                            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                            color: i === tail.length - 1 ? 'var(--text-2)' : 'var(--text-faint)',
                            fontFamily: l.mono ? 'var(--font-mono)' : 'var(--font-sans)',
                            fontSize: l.mono ? 'var(--text-caption)' : 'var(--text-control)',
                          }}>
                            {l.s}
                          </div>
                        ))}
                      </div>
                    );
                  })()}
                </div>
              </div>
              );
            })}
            {/* 任务已启动但尚无 Agent 开始流式（规划阶段）时，仍显示一个等待气泡 */}
            {activity?.phase === 'running' && Object.keys(thinking).length === 0 && (
              <div className="msg rise">
                <div className="av" style={{ width: 36, height: 36, background: 'var(--ember-tint-strong)', color: 'var(--ember-soft)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                  <Icon name="bot" size={20} />
                </div>
                <div className="msg-body">
                  <div className="typing-cap">{activity.label}</div>
                  <div className="bubble typing-bubble"><div className="typing"><i /><i /><i /></div></div>
                </div>
              </div>
            )}
          </div>

          {/* 右下角滚动控制：回到顶部 / 回到底部。0 高度锚点 + 绝对定位悬浮，自动贴在消息区底部之上 */}
          <div style={{ position: 'relative', height: 0, pointerEvents: 'none' }}>
            <div style={{ position: 'absolute', right: 16, bottom: 12, display: 'flex', flexDirection: 'column', gap: 8, pointerEvents: 'auto', zIndex: 7 }}>
              <button
                className="icon-btn"
                title="回到顶部"
                onClick={() => scrollRef.current?.scrollTo({ top: 0, behavior: 'smooth' })}
                style={{ background: 'var(--bg-2)', boxShadow: 'var(--shadow)' }}
              >
                <Icon name="arrowUp" size={16} />
              </button>
              <button
                className="icon-btn"
                title="回到底部"
                onClick={() => scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: 'smooth' })}
                style={{ background: 'var(--bg-2)', boxShadow: 'var(--shadow)' }}
              >
                <Icon name="arrowDown" size={16} />
              </button>
            </div>
          </div>

          {activity && (
            <div className={'chat-activity ' + activity.phase}>
              {activity.phase === 'running'
                ? <span className="dot amber" />
                : activity.phase === 'done'
                  ? <Icon name="check" size={13} />
                  : <Icon name="alert" size={13} />}
              {/* running 阶段实时反映真实进度：有 Agent 流式中 → 协作中；否则仍是调度安排。 */}
              <span>{activity.phase === 'running' && Object.keys(thinking).length > 0 ? '智能体协作生成中…' : activity.label}</span>
            </div>
          )}

          {loadError && (
            <div className="chat-error-bar">
              <Icon name="alert" size={14} />
              <span>操作失败：{loadError}</span>
              <button className="icon-btn" title="关闭提示" onClick={() => setLoadError('')}>
                <Icon name="x" size={13} />
              </button>
            </div>
          )}

          {/* ── Composer ── */}
          <Composer
            conv={conv}
            agents={agents}
            contextAttachments={contextAttachments}
            onSend={onSend}
            onCompress={onCompress}
            onError={setLoadError}
            quote={quoteDraft}
            onClearQuote={() => setQuoteDraft(null)}
            wsRefs={wsRefs}
            onRemoveWsRef={(path) => setWsRefs(refs => refs.filter(r => r.path !== path))}
            busy={sending}
          />
        </div>
      ) : (
        <div className="content">
          {listCollapsed && (
            <div className="chat-head">
              <button className="icon-btn" title="展开对话列表" onClick={toggleList}>
                <Icon name="columns" size={18} />
              </button>
            </div>
          )}
          <div className="empty"><Icon name="chat" /><div>选择一个会议室开始</div></div>
        </div>
      )}

      {showNew && <NewGroupModal agents={agents} projects={projects} onClose={() => setShowNew(false)} onCreate={handleNewGroup} />}
      {editGroup && <EditGroupModal conversation={editGroup} projects={projects} onClose={() => setEditGroup(null)} onSave={saveGroupInfo} />}
      {bubbleMenu && (
        <div
          className="bubble-menu"
          style={{ left: bubbleMenu.x, top: bubbleMenu.y }}
          onPointerDown={e => e.stopPropagation()}
          onContextMenu={e => e.preventDefault()}
        >
          <button onClick={copyBubbleMessage}><Icon name="copy" size={14} />复制</button>
          <button onClick={quoteBubbleMessage}><Icon name="quote" size={14} />引用</button>
          <button onClick={toggleContextBubbleMessage} title={bubbleMenu.message.excluded_from_context ? '恢复：重新加入上下文' : '排除：不进入 AI 上下文'}>
            <Icon name={bubbleMenu.message.excluded_from_context ? 'eye' : 'eye-off'} size={14} />
            {bubbleMenu.message.excluded_from_context ? '恢复' : '排除'}
          </button>
          <button onClick={openReader}><Icon name="maximize" size={14} />阅读模式</button>
          {conv?.project_id && (
            <button onClick={distillBubbleToIssue}><Icon name="inbox" size={14} />沉淀为需求</button>
          )}
          <button className="danger" onClick={deleteBubbleMessage}><Icon name="trash" size={14} />删除</button>
        </div>
      )}
      {reader && (
        <div className="reader-overlay" onClick={e => { if (e.target === e.currentTarget) setReader(null); }}>
          <div className="reader-bar" onDoubleClick={toggleMaximizeOnDoubleClick}>
            <div className="reader-bar-info">
              <Icon name="maximize" size={15} />
              <span className="reader-bar-title">{reader.author}</span>
              <span className="reader-bar-time">{fmtFull(reader.message.created_at)}</span>
            </div>
            <div className="reader-bar-tools">
              <button className="icon-btn" title="缩小字号" onClick={() => bumpReaderScale(-0.1)} disabled={readerScale <= 0.85}>
                <span style={{ fontSize: 'var(--text-label)', fontWeight: 700 }}>A−</span>
              </button>
              <span className="reader-scale-val">{Math.round(readerScale * 100)}%</span>
              <button className="icon-btn" title="放大字号" onClick={() => bumpReaderScale(0.1)} disabled={readerScale >= 2}>
                <span style={{ fontSize: 'var(--text-section)', fontWeight: 700 }}>A+</span>
              </button>
              <div className="chat-head-sep" />
              <button className="icon-btn" title="退出阅读模式 (Esc)" onClick={() => setReader(null)}>
                <Icon name="x" size={18} />
              </button>
            </div>
          </div>
          <div ref={readerScrollRef} className="reader-scroll scroll">
            <div className="bubble doc reader-doc" style={{ ['--rs' as string]: String(readerScale) }}>
              {visibleMessageBlocks(reader.message).map((b, i) => (
                <Block key={i} b={b} projectId={conv?.project_id ?? undefined} messageId={reader.message.id} blockIndex={i} />
              ))}
            </div>
            <ReaderToc scrollRef={readerScrollRef} watch={reader.message.id} />
          </div>
        </div>
      )}
      {confirmDissolve && (
        <ConfirmModal
          msg="确认解散这个群聊？解散后将删除群聊记录、成员关系和历史消息。"
          okLabel="确认解散"
          onOk={dissolveGroup}
          onCancel={() => setConfirmDissolve(null)}
        />
      )}
      {confirmArchive && (
        <ConfirmModal
          msg="确认归档当前会议室内容？当前消息将存为只读归档（可在「归档区」检索回顾），随后会议室会被清空、可继续使用；单聊归档后将从左侧列表消失。"
          okLabel="确认归档"
          onOk={archiveAndClearConversation}
          onCancel={() => setConfirmArchive(null)}
        />
      )}
      {showArchive && (
        <ArchiveBrowser agents={agents} projects={projects} onClose={() => setShowArchive(false)} />
      )}
    </>
  );
}
