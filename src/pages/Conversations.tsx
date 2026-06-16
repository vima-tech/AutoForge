import React, { useState, useRef, useEffect, useCallback, useMemo } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import { Avatar, MeAvatar } from '../components/Avatar';
import Block from '../components/Block';
import {
  listConversations, listMessages, sendMessage, createGroupConversation,
  listAgents, updateGroupConversation, addConversationMember, removeConversationMember, deleteGroupConversation,
  markConversationRead, importAttachment, listConversationAttachments, openAttachment,
  toggleMessageContext, startConversationTask,
  archiveConversation, listConversationArchives, getConversationArchive,
  searchConversationArchives, deleteConversationArchive,
  listProjectFiles, addConversationProjectContext, removeConversationProjectContext,
  listProjects, listWorkspaceFiles, readWorkspaceFile, writeWorkspaceFile, ensureWorkspaceDirs,
  runConversationCommand, INNATE_SENDER,
  type Conversation, type Message, type Agent, type ConversationAttachment,
  type Project, type ProjectContextFile, type WorkspaceFile, type ConvCommandName,
  type ConversationArchiveSummary, type ArchiveSearchHit, type ArchivedMessage,
} from '../services';
import type { BlockType } from '../data/mock';
import { fmtMsgTime, fmtListTime, fmtFull } from '../utils/datetime';

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

// 群聊 @快捷指令：`@所有人` 展开为全部可点名成员，让大家一起讨论并尽快达成一致。
const ALL_MENTION_ID = '__all__';
type MentionItem = { kind: 'all' } | { kind: 'agent'; agent: Agent };

// Innate 知识库斜杠命令：在私聊/群聊里手动触发记忆存储、召回、进化与状态查看。
interface SlashCommand { name: ConvCommandName; usage: string; desc: string; icon: string; }
const SLASH_COMMANDS: SlashCommand[] = [
  { name: 'remember', usage: '/remember [内容]', desc: '存入知识库（留空则记住最近对话）', icon: 'brain' },
  { name: 'recall',   usage: '/recall 关键词',   desc: '召回相关经验',                   icon: 'search' },
  { name: 'evolve',   usage: '/evolve',          desc: '立即蒸馏整理（进化）',           icon: 'zap' },
  { name: 'innate',   usage: '/innate',          desc: '查看知识库健康度',               icon: 'flask' },
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

function ConvList({ convs, agents, active, onSelect, onNew, onOpenArchive }: {
  convs: Conversation[]; agents: Agent[];
  active: string; onSelect: (id: string) => void; onNew: () => void; onOpenArchive: () => void;
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
  return (
    <div className="list-col">
      <div className="list-head">
        <div className="list-title-row">
          <span className="list-title">会议室</span>
          <div style={{ display: 'flex', alignItems: 'center', gap: 2 }}>
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

function MessageRow({ m, agents, isGroup, highlighted, rowRef, onBubbleContextMenu, projectId }: {
  m: Message; agents: Agent[]; isGroup: boolean;
  highlighted?: boolean; rowRef?: (el: HTMLDivElement | null) => void;
  onBubbleContextMenu?: (e: React.MouseEvent, message: Message, author: string) => void;
  projectId?: string;
}) {
  const agentMap = useMemo(() => Object.fromEntries(agents.map(a => [a.id, a])), [agents]);
  const isInnate = m.from_agent === INNATE_SENDER;
  const me = !m.from_agent;
  const a  = me || isInnate ? null : agentMap[m.from_agent!];
  const author = me ? '我' : isInnate ? 'Innate' : (a?.name ?? 'Agent');
  const blocks = visibleMessageBlocks(m);
  const quote = messageQuote(m);
  return (
    <div ref={rowRef} className={'msg' + (me ? ' me' : '') + (highlighted ? ' search-hit' : '') + ' rise'}>
      {me
        ? <MeAvatar size={36} />
        : isInnate
            ? <div className="av" style={{ width: 36, height: 36, background: 'var(--ember-tint-strong)', color: 'var(--ember-soft)', display: 'flex', alignItems: 'center', justifyContent: 'center' }} title="Innate 知识库"><Icon name="brain" size={20} /></div>
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
          className="bubble"
          onContextMenu={e => onBubbleContextMenu?.(e, m, author)}
          style={m.excluded_from_context ? { opacity: 0.45, outline: '1.5px dashed var(--border-strong)', outlineOffset: 2 } : undefined}
        >
          {blocks.map((b, i) => <Block key={i} b={b} projectId={projectId} />)}
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
      </div>
    </div>
  );
}

function Composer({ conv, agents, contextAttachments, onSend, onError, quote, onClearQuote, busy }: {
  conv: Conversation; agents: Agent[]; contextAttachments: ConversationAttachment[];
  onSend: (text: string, attachments: PendingAttachment[], contextRefs: ConversationAttachment[], mentionedAgentIds: string[]) => Promise<boolean>;
  onError: (message: string) => void;
  quote: QuoteDraft | null;
  onClearQuote: () => void;
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
  const fileInputRef = useRef<HTMLInputElement>(null);
  const imageInputRef = useRef<HTMLInputElement>(null);
  const isG = conv.conv_type === 'group';
  const agentMap = useMemo(() => Object.fromEntries(agents.map(a => [a.id, a])), [agents]);
  const members  = useMemo(
    () => isG
      ? conv.members.map(id => agentMap[id]).filter((a): a is Agent => !!a && a.mentionable && a.enabled)
      : [],
    [isG, conv.members, agentMap],
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

  useEffect(() => {
    const timer = setTimeout(() => {
      editorRef.current?.focus();
      setCaretToEnd();
    }, 0);
    return () => clearTimeout(timer);
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
    setCaretAfter(spacer);
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
    setCaretAfter(spacer);
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
    // `@所有人` 展开为当前全部可点名成员。
    const expanded = ids.flatMap(id => (id === ALL_MENTION_ID ? members.map(m => m.id) : [id]));
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
    if (!outgoing && pendingItems.length === 0 && refs.length === 0) return;
    setText('');
    setPending([]);
    if (editorRef.current) editorRef.current.innerHTML = '';
    setShowMention(false);
    setShowAttachmentPicker(false);
    await onSend(outgoing, pendingItems, refs, mentions);
  };

  const onKey = (e: React.KeyboardEvent) => {
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
      <div className="composer-tools">
        <input ref={fileInputRef} type="file" multiple accept={FILE_ACCEPT} hidden onChange={pickFiles('file')} />
        <input ref={imageInputRef} type="file" multiple accept={IMAGE_ACCEPT} hidden onChange={pickFiles('image')} />
        <button className="icon-btn" title="添加附件" disabled={busy} onClick={() => fileInputRef.current?.click()}><Icon name="paperclip" size={18} /></button>
        <button className="icon-btn" title="添加图片" disabled={busy} onClick={() => imageInputRef.current?.click()}><Icon name="image" size={18} /></button>
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
                  className={'mention-row attachment-row' + (i === attachmentSel ? ' sel' : '')}
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
          onKeyDown={onKey}
          onPaste={e => {
            e.preventDefault();
            insertPlainText(e.clipboardData.getData('text/plain'));
          }}
        />
        <button className="send-btn" disabled={(!text.trim() && pending.length === 0) || busy} onClick={send}>
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
function ArchiveMessageRow({ m, agents, isGroup, projectId }: {
  m: ArchivedMessage; agents: Agent[]; isGroup: boolean; projectId?: string;
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
  return (
    <div className={'msg' + (m.is_me ? ' me' : '')}>
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
        <div className="bubble" style={m.excluded_from_context ? { opacity: 0.45, outline: '1.5px dashed var(--border-strong)', outlineOffset: 2 } : undefined}>
          {blocks.map((b, i) => <Block key={i} b={b} projectId={projectId} />)}
        </div>
        {quote && (
          <div className="bubble-quote" title={`${quote.author}: ${quote.text}`}>
            <span className="quote-author">{quote.author}</span>
            <span className="quote-text">{quote.text}</span>
          </div>
        )}
      </div>
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
  const rows: (ConversationArchiveSummary & { match_count?: number; snippet?: string })[] =
    searching ? hits : summaries;
  const projectName = (s: ConversationArchiveSummary) =>
    s.project_name ?? (s.project_id ? projects.find(p => p.id === s.project_id)?.name : undefined);
  const isGroupMeta = (selectedMeta?.conv_type ?? 'group') === 'group';

  return (
    <div style={{ position: 'fixed', inset: 'var(--win-gutter,0)', borderRadius: 14, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 240 }}>
      <div style={{ background: 'var(--bg-1)', border: '1px solid var(--border-strong)', borderRadius: 14, width: 'min(980px, 92vw)', height: 'min(680px, 88vh)', display: 'flex', flexDirection: 'column', overflow: 'hidden', boxShadow: 'var(--shadow-lg)' }} onClick={e => e.stopPropagation()}>
        <div className="panel-head" style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '13px 16px', borderBottom: '1px solid var(--border)' }}>
          <Icon name="inbox" size={18} style={{ color: 'var(--ember)' }} />
          <div style={{ flex: 1 }}>
            <div style={{ fontFamily: 'var(--font-display)', fontSize: 'var(--text-section)', fontWeight: 700 }}>归档区</div>
            <div style={{ fontSize: 'var(--text-caption)', color: 'var(--text-3)', fontFamily: 'var(--font-mono)' }}>READ-ONLY · 只读快照检索与回顾</div>
          </div>
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
                <div className="msgs scroll" style={{ flex: 1 }}>
                  {loading
                    ? <div className="empty" style={{ color: 'var(--text-3)', padding: 30 }}>加载中…</div>
                    : messages.map((m, i) => (
                        <ArchiveMessageRow key={i} m={m} agents={agents} isGroup={isGroupMeta} projectId={selectedMeta.project_id ?? undefined} />
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
  const [active,         setActive]         = useState('');
  const [msgs,           setMsgs]           = useState<Message[]>([]);
  const [showNew,        setShowNew]        = useState(false);
  const [editGroup,      setEditGroup]      = useState<Conversation | null>(null);
  const [showMembers,    setShowMembers]    = useState(false);
  const [showContext,    setShowContext]     = useState(false);
  const [showSearch,     setShowSearch]     = useState(false);
  const [showHeadMore,   setShowHeadMore]   = useState(false);
  const [projectFiles,   setProjectFiles]   = useState<ProjectContextFile[]>([]);
  const [workspaceFiles, setWorkspaceFiles] = useState<WorkspaceFile[]>([]);
  const [workspaceTab,   setWorkspaceTab]   = useState<'docs' | 'specs'>('docs');
  const [wsFileContent,  setWsFileContent]  = useState<{ path: string; content: string } | null>(null);
  const [searchQuery,    setSearchQuery]    = useState('');
  const [activeSearchId, setActiveSearchId] = useState<string | null>(null);
  const [confirmDissolve,setConfirmDissolve]= useState<string | null>(null);
  const [confirmArchive, setConfirmArchive] = useState<string | null>(null);
  const [showArchive,    setShowArchive]    = useState(false);
  const [memberError,    setMemberError]    = useState('');
  const [sending,        setSending]        = useState(false);
  const [loadError,      setLoadError]      = useState('');
  const [quoteDraft,     setQuoteDraft]     = useState<QuoteDraft | null>(null);
  const [bubbleMenu,     setBubbleMenu]     = useState<BubbleMenuState | null>(null);
  const [contextAttachments, setContextAttachments] = useState<ConversationAttachment[]>([]);
  const [windowSize,         setWindowSize]         = useState(20);

  const scrollRef       = useRef<HTMLDivElement>(null);
  const headerActionsRef= useRef<HTMLDivElement>(null);
  const searchInputRef  = useRef<HTMLInputElement>(null);
  const messageRefs     = useRef<Record<string, HTMLDivElement | null>>({});

  // Ref keeps the event listener closure up-to-date without re-registering it.
  const activeRef = useRef(active);
  activeRef.current = active;

  // ── Stable data-fetching callbacks ─────────────────────────────────────────

  const loadConvs = useCallback(async () => {
    const [cs, as, ps] = await Promise.all([listConversations(), listAgents(), listProjects()]);
    setConvs(cs);
    setAgents(as);
    setProjects(ps);
    // Only set active when it is still empty (first load).
    // Prefer first group conversation to match the UI order (groups listed before directs).
    setActive(cur => cur || cs.find(c => c.conv_type === 'group')?.id || cs[0]?.id || '');
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

    let alive = true;
    Promise.all([loadMsgs(active), loadContextAttachments(active)]).then(() => {
      if (!alive) return;
      setConvs(cs => cs.map(c => c.id === active ? { ...c, unread: 0 } : c));
      setLoadError('');
    }).catch(e => { if (alive) setLoadError(String(e)); });

    const readTimer = setTimeout(() => {
      markConversationRead(active)
        .then(() => window.dispatchEvent(new Event('AutoForge:badges-refresh')))
        .catch(() => {});
    }, 500);

    return () => { alive = false; clearTimeout(readTimer); };
  }, [active, loadMsgs, loadContextAttachments]);

  // 3. Reset search state when switching conversations.
  useEffect(() => {
    setShowSearch(false);
    setSearchQuery('');
    setActiveSearchId(null);
    setQuoteDraft(null);
    setBubbleMenu(null);
    setEditGroup(null);
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

    listen<unknown>('AutoForge://event', e => {
      const ev = e.payload as { type?: string; conversation_id?: string };
      if (ev?.type !== 'message_received' && ev?.type !== 'conversation_task_updated') return;
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
  }, [loadConvs, loadMsgs]); // stable callbacks — this effect runs once

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
    if (!showContext) { setProjectFiles([]); setWorkspaceFiles([]); setWsFileContent(null); return; }
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

  const normalizedQ = searchQuery.trim().toLowerCase();
  const visibleMsgCount = useMemo(() => msgs.filter(m => !m.id.startsWith('typing-')).length, [msgs]);
  const searchResults = useMemo(() => {
    if (!showSearch || !normalizedQ) return [];
    return msgs
      .filter(m => !m.id.startsWith('typing-') && msgText(m).toLowerCase().includes(normalizedQ))
      .map(m => ({ message: m, text: msgText(m).replace(/\s+/g, ' ').trim(), sender: m.from_agent ? (agentMap[m.from_agent]?.name ?? 'Agent') : '我' }));
  }, [msgs, showSearch, normalizedQ, agentMap]);

  // ── Actions ────────────────────────────────────────────────────────────────

  const jumpToMessage = (id: string) => {
    setActiveSearchId(id);
    messageRefs.current[id]?.scrollIntoView({ block: 'center', behavior: 'smooth' });
  };

  const openBubbleMenu = (e: React.MouseEvent, message: Message, author: string) => {
    e.preventDefault();
    setBubbleMenu({
      x: Math.min(e.clientX, window.innerWidth - 132),
      y: Math.min(e.clientY, window.innerHeight - 96),
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

  const onSend = async (
    text: string,
    attachments: PendingAttachment[],
    contextRefs: ConversationAttachment[],
    mentionedAgentIds: string[],
  ) => {
    if (!conv || sending) return false;
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
      setQuoteDraft(null);
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

      const shouldStartTask = text.trim().length > 0 || contextRefs.length > 0 || attachments.length > 0;
      if (shouldStartTask) {
        const directAgentIds = conv.conv_type === 'direct'
          ? conv.members.filter(id => {
              const a = agentMap[id];
              return !!a && a.enabled;
            }).slice(0, 1)
          : [];
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
      setWsFileContent(null);
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
      <ConvList convs={convs} agents={agents} active={active} onSelect={setActive} onNew={() => setShowNew(true)} onOpenArchive={() => setShowArchive(true)} />

      {conv ? (
        <div className="content">
          {/* ── Chat header ── */}
          <div className="chat-head">
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
                <div className="mention-pop" style={{ right: 0, left: 'auto', top: 38, bottom: 'auto', width: 200 }}>
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
                          {(['docs', 'specs'] as const).map(tab => (
                            <button key={tab} onClick={() => { setWorkspaceTab(tab); setWsFileContent(null); }}
                              className="btn btn-sm"
                              style={{ padding: '1px 8px', fontSize: 'var(--text-micro)', background: workspaceTab === tab ? 'var(--ember)' : undefined, color: workspaceTab === tab ? '#fff' : undefined }}>
                              {tab}
                            </button>
                          ))}
                        </div>
                      </div>
                      {/* File viewer */}
                      {wsFileContent && (
                        <div style={{ margin: '4px 8px 4px', border: '1px solid var(--border)', borderRadius: 8, overflow: 'hidden' }}>
                          <div style={{ padding: '6px 10px', background: 'var(--bg-3)', display: 'flex', alignItems: 'center', gap: 6, fontSize: 'var(--text-caption)' }}>
                            <Icon name="file" size={11} style={{ color: 'var(--ember)' }} />
                            <span style={{ fontFamily: 'var(--font-mono)', flex: 1, color: 'var(--text-2)' }}>.autoforge/{wsFileContent.path}</span>
                            <button className="icon-btn" style={{ width: 20, height: 20 }} onClick={() => setWsFileContent(null)}>
                              <Icon name="x" size={11} />
                            </button>
                          </div>
                          <pre style={{ margin: 0, padding: '8px 10px', fontSize: 'var(--text-caption)', lineHeight: 'var(--leading-relaxed)', color: 'var(--text-2)', maxHeight: 220, overflowY: 'auto', whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>
                            {wsFileContent.content}
                          </pre>
                        </div>
                      )}
                      {/* File list */}
                      {workspaceFiles.filter(f => f.subfolder === workspaceTab).length === 0 ? (
                        <div className="empty-compact" style={{ padding: '8px 8px' }}>
                          .autoforge/{workspaceTab}/ 暂无文件
                          <span style={{ display: 'block', fontSize: 'var(--text-micro)', color: 'var(--text-faint)', marginTop: 3 }}>
                            让 Agent 创建文档，或直接点击 Artifact 的"存入 {workspaceTab}"按钮
                          </span>
                        </div>
                      ) : (
                        workspaceFiles.filter(f => f.subfolder === workspaceTab).map(f => (
                          <div key={f.rel_path} className="mention-row" style={{ cursor: 'pointer' }}
                            onClick={() => {
                              if (wsFileContent?.path === f.rel_path) { setWsFileContent(null); return; }
                              if (conv.project_id) {
                                readWorkspaceFile(conv.project_id, f.rel_path)
                                  .then(content => setWsFileContent({ path: f.rel_path, content }))
                                  .catch(err => setLoadError(String(err)));
                              }
                            }}>
                            <div className="cfg-logo" style={{ width: 28, height: 28, background: 'var(--ember)', flexShrink: 0 }}>
                              <Icon name="file" size={13} />
                            </div>
                            <div style={{ minWidth: 0, flex: 1 }}>
                              <div className="nm">{f.name}</div>
                              <div className="rl" style={{ fontSize: 'var(--text-caption)' }}>{f.modified_at} · {(f.size_bytes / 1024).toFixed(1)} KB</div>
                            </div>
                            <Icon name={wsFileContent?.path === f.rel_path ? 'chevronUp' : 'chevronDown'} size={12} style={{ color: 'var(--text-3)', flexShrink: 0 }} />
                          </div>
                        ))
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
                        只读文件仅注入上下文供参考，可写范围仅限 .autoforge/docs/ 和 .autoforge/specs/
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
          <div className="msgs scroll" ref={scrollRef}>
            {msgs.map((m, i) => (
              <MessageRow
                key={m.id ?? i}
                m={m}
                agents={agents}
                isGroup={conv.conv_type === 'group'}
                highlighted={activeSearchId === m.id}
                rowRef={el => { messageRefs.current[m.id] = el; }}
                onBubbleContextMenu={openBubbleMenu}
                projectId={conv.project_id ?? undefined}
              />
            ))}
          </div>

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
            onError={setLoadError}
            quote={quoteDraft}
            onClearQuote={() => setQuoteDraft(null)}
            busy={sending}
          />
        </div>
      ) : (
        <div className="content">
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
