import React, { useState, useRef, useEffect, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import Icon from '../components/Icon';
import { Avatar, MeAvatar } from '../components/Avatar';
import Block from '../components/Block';
import {
  listConversations, listMessages, sendMessage, createGroupConversation, agentReply,
  listAgents, addConversationMember, removeConversationMember,
  type Conversation, type Message, type Agent,
} from '../services';
import type { BlockType } from '../data/mock';

function ConvList({ convs, agents, active, onSelect, onNew }: {
  convs: Conversation[]; agents: Agent[];
  active: string; onSelect: (id: string) => void; onNew: () => void;
}) {
  const [q, setQ] = useState('');
  const agentMap = Object.fromEntries(agents.map(a => [a.id, a]));
  const groups  = convs.filter(c => c.conv_type === 'group');
  const directs = convs.filter(c => c.conv_type === 'direct');
  const title = (c: Conversation) => c.conv_type === 'group' ? (c.name ?? '群聊') : (agentMap[c.members[0]]?.name ?? 'Agent');
  const match = (c: Conversation) => !q || title(c).toLowerCase().includes(q.toLowerCase());
  const preview = (c: Conversation) => {
    if (!c.last_message) return '暂无消息';
    try { const bs: BlockType[] = JSON.parse(c.last_message); return bs[0]?.t === 'md' ? bs[0].md.slice(0, 40) : '消息'; }
    catch { return c.last_message.slice(0, 40); }
  };

  const Item = (c: Conversation) => {
    const isG = c.conv_type === 'group';
    const a = isG ? null : agentMap[c.members[0]];
    return (
      <div key={c.id} className={'conv-item' + (active === c.id ? ' active' : '')} onClick={() => onSelect(c.id)}>
        {isG
          ? <div className="av sq" style={{ width: 46, height: 46, background: c.color, fontSize: 18 }}>{c.initial ?? c.name?.[0] ?? '群'}</div>
          : a ? <Avatar agent={a} size={46} status="online" /> : <div className="av" style={{ width: 46, height: 46, background: '#888' }}>?</div>}
        <div className="conv-main">
          <div className="conv-top">
            <span className="conv-name">{title(c)}</span>
            <span className="conv-time">{c.last_time ? new Date(c.last_time).toLocaleTimeString('zh',{hour:'2-digit',minute:'2-digit'}) : ''}</span>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <span className="conv-preview">{isG && <Icon name="bot" size={11} style={{ verticalAlign: -1, marginRight: 3, color: 'var(--text-faint)' }} />}{preview(c)}</span>
            {c.unread > 0 && <span className="conv-unread" style={{ marginLeft: 'auto' }}>{c.unread}</span>}
          </div>
        </div>
      </div>
    );
  };

  return (
    <div className="list-col">
      <div className="list-head">
        <div className="list-title-row">
          <span className="list-title">对话</span>
          <button className="icon-btn" title="新建群聊" onClick={onNew} style={{ color: 'var(--ember)' }}><Icon name="plus" size={20} /></button>
        </div>
        <div className="search"><Icon name="search" size={15} /><input placeholder="搜索 Agent 或群聊" value={q} onChange={e => setQ(e.target.value)} /></div>
      </div>
      <div className="list-body scroll">
        <div className="list-group-label">群聊 · 需求讨论</div>
        {groups.filter(match).map(Item)}
        <div className="list-group-label">Agent · 单独对话</div>
        {directs.filter(match).map(Item)}
      </div>
    </div>
  );
}

function MessageRow({ m, agents, isGroup }: { m: Message; agents: Agent[]; isGroup: boolean }) {
  const agentMap = Object.fromEntries(agents.map(a => [a.id, a]));
  const me = !m.from_agent;
  const a = me ? null : agentMap[m.from_agent!];
  let blocks: BlockType[] = [];
  try { blocks = JSON.parse(m.content_json); } catch { blocks = [{ t: 'md', md: m.content_json }]; }
  return (
    <div className={'msg' + (me ? ' me' : '') + ' rise'}>
      {me ? <MeAvatar size={36} /> : (a ? <Avatar agent={a} size={36} /> : <div className="av" style={{ width:36,height:36,background:'#888',fontSize:14 }}>?</div>)}
      <div className="msg-body">
        {!me && a && (
          <div className="msg-meta">
            <span className="msg-author" style={{ color: a.color }}>{a.name}</span>
            {isGroup && <span className="chip" style={{ padding: '0px 6px', fontSize: 9.5 }}>{a.name_en}</span>}
            <span className="msg-time">{new Date(m.created_at).toLocaleTimeString('zh',{hour:'2-digit',minute:'2-digit'})}</span>
          </div>
        )}
        <div className="bubble">
          {blocks.map((b, i) => <Block key={i} b={b} />)}
        </div>
      </div>
    </div>
  );
}

function Composer({ conv, agents, onSend }: { conv: Conversation; agents: Agent[]; onSend: (text: string) => void }) {
  const [text, setText] = useState('');
  const [showMention, setShowMention] = useState(false);
  const [mentionSel, setMentionSel] = useState(0);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const isG = conv.conv_type === 'group';
  const agentMap = Object.fromEntries(agents.map(a => [a.id, a]));
  const members = isG ? conv.members.map(id => agentMap[id]).filter(Boolean) : [];

  const onChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const v = e.target.value;
    setText(v);
    if (isG && /@[^\s]*$/.test(v)) { setShowMention(true); setMentionSel(0); } else setShowMention(false);
    const ta = taRef.current; if (ta) { ta.style.height = 'auto'; ta.style.height = Math.min(ta.scrollHeight, 140) + 'px'; }
  };
  const pickMention = (a: Agent) => {
    setText(t => t.replace(/@[^\s]*$/, '@' + a.name + ' '));
    setShowMention(false); taRef.current?.focus();
  };
  const send = () => { if (!text.trim()) return; onSend(text.trim()); setText(''); if (taRef.current) taRef.current.style.height = 'auto'; };
  const onKey = (e: React.KeyboardEvent) => {
    if (showMention && members.length > 0) {
      if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); pickMention(members[mentionSel]); return; }
      if (e.key === 'ArrowDown') { e.preventDefault(); setMentionSel(s => (s+1)%members.length); return; }
      if (e.key === 'ArrowUp')   { e.preventDefault(); setMentionSel(s => (s-1+members.length)%members.length); return; }
    }
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); }
  };
  const agentName = isG ? '群聊' : (agents.find(a => conv.members.includes(a.id))?.name ?? 'Agent');

  return (
    <div className="composer">
      <div className="composer-tools">
        <button className="icon-btn" title="添加附件"><Icon name="paperclip" size={18} /></button>
        <button className="icon-btn" title="发送图片"><Icon name="image" size={18} /></button>
        {isG && <button className="icon-btn" title="@ 指定 Agent" onClick={() => { setText(t => t+'@'); setShowMention(true); taRef.current?.focus(); }}><Icon name="at" size={18} /></button>}
        <div style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', paddingRight: 4 }}>
          {isG ? '群聊共享上下文' : 'Enter 发送'}
        </div>
      </div>
      <div className="composer-box" style={{ position: 'relative' }}>
        {showMention && members.length > 0 && (
          <div className="mention-pop">
            <div className="mention-pop-label">@ 指定 Agent 回答</div>
            {members.map((a, i) => (
              <div key={a.id} className={'mention-row' + (i===mentionSel?' sel':'')} onMouseEnter={() => setMentionSel(i)} onClick={() => pickMention(a)}>
                <Avatar agent={a} size={30} /><div><div className="nm">{a.name}</div><div className="rl">{a.role}</div></div>
              </div>
            ))}
          </div>
        )}
        <textarea ref={taRef} rows={1} value={text} onChange={onChange} onKeyDown={onKey}
          placeholder={isG ? '输入消息，@ 可指定 Agent 回答…' : `给 ${agentName} 发消息…`} />
        <button className="send-btn" disabled={!text.trim()} onClick={send}><Icon name="send" size={18} /></button>
      </div>
    </div>
  );
}

function NewGroupModal({ agents, onClose, onCreate }: { agents: Agent[]; onClose: () => void; onCreate: (name: string, ids: string[]) => void }) {
  const [sel, setSel] = useState<string[]>([]);
  const [name, setName] = useState('');
  const toggle = (id: string) => setSel(s => s.includes(id) ? s.filter(x => x!==id) : [...s,id]);
  return (
    <div style={{ position: 'absolute', inset: 0, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 60 }} onClick={onClose}>
      <div style={{ width: 420, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center' }}>
          <div><div className="eyebrow" style={{ fontSize: 16 }}><span className="cn">新建群聊</span></div>
            <div style={{ fontSize: 12.5, color: 'var(--text-3)', marginTop: 4 }}>拉多个 Agent 进入群聊，共享上下文协同讨论需求</div></div>
          <button className="icon-btn" style={{ marginLeft: 'auto' }} onClick={onClose}><Icon name="x" size={18} /></button>
        </div>
        <div style={{ padding: '16px 20px' }}>
          <div className="field" style={{ marginBottom: 16 }}><label>群聊名称</label>
            <input value={name} onChange={e => setName(e.target.value)} placeholder="例如：Vocant · 导出性能优化" />
          </div>
          <div className="field"><label>选择 Agent（{sel.length}）</label></div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6, marginTop: 8 }}>
            {agents.map(a => (
              <div key={a.id} className="mention-row" style={{ border: '1px solid '+(sel.includes(a.id)?'var(--ember)':'transparent'), background: sel.includes(a.id)?'var(--ember-tint)':'transparent' }} onClick={() => toggle(a.id)}>
                <Avatar agent={a} size={34} /><div style={{ flex: 1 }}><div className="nm">{a.name}</div><div className="rl">{a.role}</div></div>
                {sel.includes(a.id) && <Icon name="check" size={14} style={{ color: 'var(--ember)' }} />}
              </div>
            ))}
          </div>
        </div>
        <div style={{ padding: '14px 20px', borderTop: '1px solid var(--border)', display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn btn-primary" disabled={sel.length < 2 || !name.trim()} onClick={() => onCreate(name, sel)}>
            <Icon name="plus" size={15} />创建群聊
          </button>
        </div>
      </div>
    </div>
  );
}

export default function ConversationsPage() {
  const [convs, setConvs] = useState<Conversation[]>([]);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [active, setActive] = useState('');
  const [msgs, setMsgs] = useState<Message[]>([]);
  const [showNew, setShowNew] = useState(false);
  const [showMembers, setShowMembers] = useState(false);
  const [showContext, setShowContext] = useState(false);
  const [memberError, setMemberError] = useState('');
  const [sending, setSending] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const headerActionsRef = useRef<HTMLDivElement>(null);

  const loadConvs = useCallback(async () => {
    const [cs, as] = await Promise.all([listConversations(), listAgents()]);
    setConvs(cs); setAgents(as);
    if (!active && cs.length > 0) setActive(cs[0].id);
  }, [active]);

  const loadMsgs = useCallback(async (cid: string) => {
    if (!cid) return;
    const ms = await listMessages(cid);
    setMsgs(ms);
    setTimeout(() => { if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight; }, 50);
  }, []);

  useEffect(() => { loadConvs(); }, []);
  useEffect(() => { if (active) loadMsgs(active); }, [active, loadMsgs]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ conversation_id: string }>('autoforge://event', e => {
      const ev = e.payload as { type?: string; conversation_id?: string };
      if (ev?.type === 'message_received' && ev.conversation_id === active) loadMsgs(active);
      loadConvs();
    }).then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, [active, loadConvs, loadMsgs]);

  useEffect(() => {
    if (!showMembers && !showContext) return;

    const closeIfOutside = (e: PointerEvent) => {
      const target = e.target;
      if (!(target instanceof Node)) return;
      if (headerActionsRef.current?.contains(target)) return;
      setShowMembers(false);
      setShowContext(false);
      setMemberError('');
    };

    document.addEventListener('pointerdown', closeIfOutside);
    return () => document.removeEventListener('pointerdown', closeIfOutside);
  }, [showMembers, showContext]);

  const conv = convs.find(c => c.id === active);
  const agentMap = Object.fromEntries(agents.map(a => [a.id, a]));
  const convMembers = conv ? conv.members.map(id => agentMap[id]).filter(Boolean) : [];
  const availableAgents = conv ? agents.filter(a => !conv.members.includes(a.id)) : [];
  const memberNames = convMembers.map(a => a.name).join(' · ');
  const chatHeadSub = memberNames.length > 40 ? memberNames.slice(0, 40) + '...' : memberNames;
  const contextBlocks = msgs.flatMap(m => {
    try {
      const blocks: BlockType[] = JSON.parse(m.content_json);
      return blocks.filter(b => b.t === 'file' || b.t === 'image' || b.t === 'artifact' || b.t === 'code');
    } catch {
      return [];
    }
  });

  const onSend = async (text: string) => {
    if (!conv || sending) return;
    setSending(true);
    const m = await sendMessage({ conversation_id: conv.id, content_json: JSON.stringify([{ t: 'md', md: text }]) });
    setMsgs(ms => [...ms, m]);
    setTimeout(() => { if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight; }, 50);

    // Find which agent should respond
    let respondAgentId: string | null = null;
    if (conv.conv_type === 'direct') {
      respondAgentId = conv.members[0] ?? null;
    } else {
      const mention = text.match(/@([^\s，。@]+)/);
      if (mention) {
        const found = agents.find(a => mention[1].startsWith(a.name.slice(0, 2)));
        respondAgentId = found?.id ?? conv.members[0] ?? null;
      } else {
        respondAgentId = conv.members[0] ?? null;
      }
    }
    if (respondAgentId) {
      // Show typing indicator
      const typingId = 'typing-' + Date.now();
      setMsgs(ms => [...ms, { id: typingId, conversation_id: conv.id, from_agent: respondAgentId, content_json: JSON.stringify([{t:'typing'}]), created_at: new Date().toISOString() }]);
      try {
        await agentReply(conv.id, respondAgentId);
        // Reload messages after reply
        await loadMsgs(conv.id);
      } catch {
        setMsgs(ms => ms.filter(m => m.id !== typingId));
      }
    }
    setSending(false);
  };

  const handleNewGroup = async (name: string, memberIds: string[]) => {
    const c = await createGroupConversation(name, memberIds);
    // Re-fetch to get members populated
    await loadConvs();
    setActive(c.id);
    setShowNew(false);
  };

  const replaceConversation = (updated: Conversation) => {
    setConvs(cs => cs.map(c => c.id === updated.id ? updated : c));
  };

  const addMember = async (agentId: string) => {
    if (!conv) return;
    setMemberError('');
    try {
      const updated = await addConversationMember(conv.id, agentId);
      replaceConversation(updated);
    } catch (e) {
      setMemberError(String(e));
    }
  };

  const removeMember = async (agentId: string) => {
    if (!conv) return;
    setMemberError('');
    try {
      const updated = await removeConversationMember(conv.id, agentId);
      replaceConversation(updated);
    } catch (e) {
      setMemberError(String(e));
    }
  };

  return (
    <>
      <ConvList convs={convs} agents={agents} active={active} onSelect={id => setActive(id)} onNew={() => setShowNew(true)} />
      {conv
        ? <div className="content">
            <div className="chat-head">
              {conv.conv_type === 'group'
                ? <div className="av sq" style={{ width: 38, height: 38, background: conv.color, fontSize: 15 }}>{conv.initial ?? conv.name?.[0] ?? '群'}</div>
                : (() => { const a = agents.find(ag => conv.members.includes(ag.id)); return a ? <Avatar agent={a} size={38} status="online" /> : null; })()}
              <div className="chat-head-info">
                <div className="chat-head-title">
                  {conv.conv_type === 'group' ? conv.name : agents.find(a => conv.members.includes(a.id))?.name ?? 'Agent'}
                  {conv.conv_type === 'group' && <span className="chip" style={{ padding: '1px 7px', fontSize: 10 }}>{conv.members.length} 成员</span>}
                </div>
                <div className="chat-head-sub">{chatHeadSub}</div>
              </div>
              <div className="chat-head-actions" ref={headerActionsRef} style={{ position: 'relative' }}>
                {conv.conv_type === 'group' && (
                  <button className="member-stack" title="群成员列表" onClick={() => { setShowMembers(v => !v); setShowContext(false); }} style={{ background: 'transparent', border: 0, padding: '0 4px', cursor: 'pointer' }}>
                    {convMembers.slice(0, 4).map(a => <Avatar key={a.id} agent={a} size={28} />)}
                  </button>
                )}
                <button className="icon-btn" title="群成员列表" onClick={() => { setShowMembers(v => !v); setShowContext(false); }}><Icon name="users" size={18} /></button>
                <button className="icon-btn" title="对话上下文与附件" onClick={() => { setShowContext(v => !v); setShowMembers(false); }}><Icon name="layers" size={18} /></button>
                <button className="icon-btn" title="搜索对话"><Icon name="search" size={18} /></button>
                {showMembers && (
                  <div className="mention-pop" style={{ right: 0, left: 'auto', top: 38, bottom: 'auto', width: 280 }}>
                    <div className="mention-pop-label">{conv.conv_type === 'group' ? '群成员' : '对话成员'}</div>
                    {memberError && <div style={{ padding: '6px 8px', color: 'var(--red)', fontSize: 12 }}>{memberError}</div>}
                    {convMembers.map(a => (
                      <div key={a.id} className="mention-row">
                        <Avatar agent={a} size={32} />
                        <div style={{ minWidth: 0, flex: 1 }}>
                          <div className="nm">{a.name}</div>
                          <div className="rl">{a.role || a.name_en}</div>
                        </div>
                        {conv.conv_type === 'group' && convMembers.length > 2 && (
                          <button className="icon-btn" title="移除成员" style={{ width: 26, height: 26 }} onClick={(e) => { e.stopPropagation(); removeMember(a.id); }}>
                            <Icon name="x" size={13} />
                          </button>
                        )}
                      </div>
                    ))}
                    {convMembers.length === 0 && <div style={{ padding: '10px 8px', color: 'var(--text-3)', fontSize: 13 }}>暂无成员信息</div>}
                    {conv.conv_type === 'group' && (
                      <>
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
                        {availableAgents.length === 0 && <div style={{ padding: '8px', color: 'var(--text-3)', fontSize: 12 }}>所有 Agent 均已在群内</div>}
                      </>
                    )}
                  </div>
                )}
                {showContext && (
                  <div className="mention-pop" style={{ right: 0, left: 'auto', top: 38, bottom: 'auto', width: 320 }}>
                    <div className="mention-pop-label">对话上下文与附件</div>
                    <div style={{ padding: '7px 8px 9px', color: 'var(--text-3)', fontSize: 12, lineHeight: 1.5 }}>
                      消息 {msgs.length} 条 · 上下文块 {contextBlocks.length} 个
                    </div>
                    {contextBlocks.slice(-8).reverse().map((b, i) => (
                      <div key={i} className="mention-row" style={{ alignItems: 'flex-start' }}>
                        <div className="cfg-logo" style={{ width: 30, height: 30, background: b.t === 'image' ? 'var(--blue)' : b.t === 'file' ? 'var(--amber)' : 'var(--ember)' }}>
                          <Icon name={b.t === 'image' ? 'image' : b.t === 'file' ? 'file' : b.t === 'code' ? 'code' : 'zap'} size={15} />
                        </div>
                        <div style={{ minWidth: 0 }}>
                          <div className="nm">
                            {b.t === 'file' ? b.name : b.t === 'image' ? b.label : b.t === 'code' ? `${b.lang} 代码片段` : b.title}
                          </div>
                          <div className="rl">
                            {b.t === 'file' || b.t === 'image' ? b.meta : b.t === 'artifact' ? b.kind : '可引用上下文'}
                          </div>
                        </div>
                      </div>
                    ))}
                    {contextBlocks.length === 0 && <div style={{ padding: '10px 8px', color: 'var(--text-3)', fontSize: 13 }}>暂无附件或上下文块</div>}
                  </div>
                )}
              </div>
            </div>
            <div className="msgs scroll" ref={scrollRef}>
              {msgs.map((m, i) => <MessageRow key={m.id ?? i} m={m} agents={agents} isGroup={conv.conv_type === 'group'} />)}
            </div>
            <Composer conv={conv} agents={agents} onSend={onSend} />
          </div>
        : <div className="content"><div className="empty"><Icon name="chat" /><div>选择一个对话开始</div></div></div>}
      {showNew && <NewGroupModal agents={agents} onClose={() => setShowNew(false)} onCreate={handleNewGroup} />}
    </>
  );
}
