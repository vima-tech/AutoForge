import React, { useState, useRef, useEffect } from 'react';
import Icon from '../components/Icon';
import { Avatar, MeAvatar } from '../components/Avatar';
import Block from '../components/Block';
import { AGENTS, AGENT_MAP, CONVERSATIONS, MESSAGES, type Conversation, type Message } from '../data/mock';

function ConvList({ active, onSelect, onNew }: { active: string; onSelect: (id: string) => void; onNew: () => void }) {
  const [q, setQ] = useState('');
  const directs = CONVERSATIONS.filter(c => c.type === 'direct');
  const groups = CONVERSATIONS.filter(c => c.type === 'group');
  const title = (c: Conversation) => c.type === 'direct' ? AGENT_MAP[c.agent!].name : c.name!;
  const match = (c: Conversation) => !q || title(c).toLowerCase().includes(q.toLowerCase());

  const Item = (c: Conversation) => {
    const isG = c.type === 'group';
    const a = isG ? null : AGENT_MAP[c.agent!];
    return (
      <div key={c.id} className={'conv-item' + (active === c.id ? ' active' : '')} onClick={() => onSelect(c.id)}>
        {isG
          ? <div className="av sq" style={{ width: 46, height: 46, background: c.color, fontSize: 18 }}>{c.initial}</div>
          : <Avatar agent={a!} size={46} status="online" />}
        <div className="conv-main">
          <div className="conv-top">
            <span className="conv-name">{title(c)}</span>
            <span className="conv-time">{c.time}</span>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <span className="conv-preview">
              {isG && <Icon name="bot" size={11} style={{ verticalAlign: -1, marginRight: 3, color: 'var(--text-faint)' }} />}
              {c.preview}
            </span>
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
          <button className="icon-btn" title="新建群聊" onClick={onNew} style={{ color: 'var(--ember)' }}>
            <Icon name="plus" size={20} />
          </button>
        </div>
        <div className="search">
          <Icon name="search" size={15} />
          <input placeholder="搜索 Agent 或群聊" value={q} onChange={e => setQ(e.target.value)} />
        </div>
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

function ChatHeader({ conv }: { conv: Conversation }) {
  const isG = conv.type === 'group';
  const members = isG ? conv.members!.map(id => AGENT_MAP[id]) : null;
  return (
    <div className="chat-head">
      {isG
        ? <div className="av sq" style={{ width: 38, height: 38, background: conv.color, fontSize: 15 }}>{conv.initial}</div>
        : <Avatar agent={conv.agent!} size={38} status="online" />}
      <div className="chat-head-info">
        <div className="chat-head-title">
          {isG ? conv.name : AGENT_MAP[conv.agent!].name}
          {isG && <span className="chip" style={{ padding: '1px 7px', fontSize: 10 }}>{conv.members!.length} 成员</span>}
        </div>
        <div className="chat-head-sub">
          {isG
            ? <span style={{ fontFamily: 'var(--font-mono)' }}>{members!.map(m => m.name).join(' · ')}</span>
            : <>{AGENT_MAP[conv.agent!].en} · {AGENT_MAP[conv.agent!].llm} · <span style={{ color: 'var(--green-soft)' }}>在线</span></>}
        </div>
      </div>
      <div className="chat-head-actions">
        {isG && (
          <div className="member-stack" style={{ marginRight: 8 }}>
            {members!.slice(0, 4).map((m, i) => <Avatar key={i} agent={m} size={28} />)}
          </div>
        )}
        <button className="icon-btn" title="对话上下文与附件"><Icon name="layers" size={18} /></button>
        <button className="icon-btn"><Icon name="search" size={18} /></button>
      </div>
    </div>
  );
}

function MessageRow({ m, isGroup }: { m: Message; isGroup: boolean }) {
  const me = m.from === 'me';
  const a = me ? null : AGENT_MAP[m.from];
  return (
    <div className={'msg' + (me ? ' me' : '') + ' rise'}>
      {me ? <MeAvatar size={36} /> : <Avatar agent={a!} size={36} />}
      <div className="msg-body">
        {!me && (
          <div className="msg-meta">
            <span className="msg-author" style={{ color: a!.color }}>{a!.name}</span>
            {isGroup && <span className="chip" style={{ padding: '0px 6px', fontSize: 9.5 }}>{a!.en}</span>}
            <span className="msg-time">{m.time}</span>
          </div>
        )}
        <div className="bubble">
          {m.blocks.map((b, i) => <Block key={i} b={b} />)}
        </div>
      </div>
    </div>
  );
}

function Composer({ conv, onSend }: { conv: Conversation; onSend: (text: string) => void }) {
  const [text, setText] = useState('');
  const [showMention, setShowMention] = useState(false);
  const [mentionSel, setMentionSel] = useState(0);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const isG = conv.type === 'group';
  const members = isG ? conv.members!.map(id => AGENT_MAP[id]) : [];

  const onChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const v = e.target.value;
    setText(v);
    if (isG && /@[^\s]*$/.test(v)) { setShowMention(true); setMentionSel(0); }
    else setShowMention(false);
    const ta = taRef.current;
    if (ta) { ta.style.height = 'auto'; ta.style.height = Math.min(ta.scrollHeight, 140) + 'px'; }
  };

  const pickMention = (agentId: string) => {
    const a = AGENT_MAP[agentId];
    setText(t => t.replace(/@[^\s]*$/, '@' + a.name + ' '));
    setShowMention(false);
    taRef.current?.focus();
  };

  const send = () => {
    if (!text.trim()) return;
    onSend(text.trim());
    setText('');
    if (taRef.current) taRef.current.style.height = 'auto';
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (showMention && (e.key === 'Enter' || e.key === 'Tab')) { e.preventDefault(); pickMention(members[mentionSel].id); return; }
    if (showMention && e.key === 'ArrowDown') { e.preventDefault(); setMentionSel(s => (s + 1) % members.length); return; }
    if (showMention && e.key === 'ArrowUp') { e.preventDefault(); setMentionSel(s => (s - 1 + members.length) % members.length); return; }
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); }
  };

  return (
    <div className="composer">
      <div className="composer-tools">
        <button className="icon-btn" title="添加附件"><Icon name="paperclip" size={18} /></button>
        <button className="icon-btn" title="发送图片"><Icon name="image" size={18} /></button>
        {isG && (
          <button className="icon-btn" title="@ 指定 Agent" onClick={() => { setText(t => t + '@'); setShowMention(true); taRef.current?.focus(); }}>
            <Icon name="at" size={18} />
          </button>
        )}
        <button className="icon-btn" title="引用上下文"><Icon name="layers" size={18} /></button>
        <div style={{ marginLeft: 'auto', alignSelf: 'center', fontSize: 11, color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', paddingRight: 4 }}>
          {isG ? '群聊共享上下文与附件' : 'Enter 发送 · Shift+Enter 换行'}
        </div>
      </div>
      <div className="composer-box" style={{ position: 'relative' }}>
        {showMention && (
          <div className="mention-pop">
            <div className="mention-pop-label">@ 指定 Agent 回答</div>
            {members.map((a, i) => (
              <div key={a.id} className={'mention-row' + (i === mentionSel ? ' sel' : '')} onMouseEnter={() => setMentionSel(i)} onClick={() => pickMention(a.id)}>
                <Avatar agent={a} size={30} />
                <div><div className="nm">{a.name}</div><div className="rl">{a.role}</div></div>
              </div>
            ))}
          </div>
        )}
        <textarea
          ref={taRef}
          rows={1}
          value={text}
          onChange={onChange}
          onKeyDown={onKey}
          placeholder={isG ? '输入消息，@ 可指定 Agent 回答…' : `给 ${AGENT_MAP[conv.agent!]?.name ?? 'Agent'} 发消息…`}
        />
        <button className="send-btn" disabled={!text.trim()} onClick={send}>
          <Icon name="send" size={18} />
        </button>
      </div>
    </div>
  );
}

function ChatView({ conv, msgs, onSend }: { conv: Conversation; msgs: Message[]; onSend: (text: string) => void }) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const isG = conv.type === 'group';
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [msgs, conv.id]);
  return (
    <div className="content">
      <ChatHeader conv={conv} />
      <div className="msgs scroll" ref={scrollRef}>
        <div className="day-sep">今天</div>
        {msgs.map((m, i) => <MessageRow key={i} m={m} isGroup={isG} />)}
      </div>
      <Composer conv={conv} onSend={onSend} />
    </div>
  );
}

function NewGroupModal({ onClose }: { onClose: () => void }) {
  const [sel, setSel] = useState<string[]>([]);
  const [name, setName] = useState('');
  const toggle = (id: string) => setSel(s => s.includes(id) ? s.filter(x => x !== id) : [...s, id]);
  return (
    <div style={{ position: 'absolute', inset: 0, background: 'rgba(0,0,0,.5)', backdropFilter: 'blur(3px)', display: 'grid', placeItems: 'center', zIndex: 60 }} onClick={onClose}>
      <div className="rise" style={{ width: 420, background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderRadius: 18, boxShadow: 'var(--shadow-lg)', overflow: 'hidden' }} onClick={e => e.stopPropagation()}>
        <div style={{ padding: '18px 20px 14px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center' }}>
          <div>
            <div className="eyebrow" style={{ fontSize: 16 }}><span className="cn">新建群聊</span></div>
            <div style={{ fontSize: 12.5, color: 'var(--text-3)', marginTop: 4 }}>拉多个 Agent 进入群聊，共享上下文协同讨论需求</div>
          </div>
          <button className="icon-btn" style={{ marginLeft: 'auto' }} onClick={onClose}><Icon name="x" size={18} /></button>
        </div>
        <div style={{ padding: '16px 20px' }}>
          <div className="field" style={{ marginBottom: 16 }}>
            <label>群聊名称</label>
            <input value={name} onChange={e => setName(e.target.value)} placeholder="例如：Vocant · 导出性能优化" />
          </div>
          <div className="field"><label>选择 Agent（{sel.length}）</label></div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6, marginTop: 8 }}>
            {AGENTS.map(a => (
              <div key={a.id} className="mention-row" style={{ border: '1px solid ' + (sel.includes(a.id) ? 'var(--ember)' : 'transparent'), background: sel.includes(a.id) ? 'var(--ember-tint)' : 'transparent' }} onClick={() => toggle(a.id)}>
                <Avatar agent={a} size={34} />
                <div style={{ flex: 1 }}><div className="nm">{a.name}</div><div className="rl">{a.role}</div></div>
                <div style={sel.includes(a.id) ? { width: 18, height: 18, borderRadius: 6, background: 'var(--ember)', display: 'grid', placeItems: 'center' } : { width: 8, height: 8, borderRadius: '50%', background: 'var(--text-faint)' }}>
                  {sel.includes(a.id) && <Icon name="check" size={12} style={{ color: '#2a1607' }} />}
                </div>
              </div>
            ))}
          </div>
        </div>
        <div style={{ padding: '14px 20px', borderTop: '1px solid var(--border)', display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn btn-primary" disabled={sel.length < 2} style={sel.length < 2 ? { opacity: .5 } : {}} onClick={onClose}>
            <Icon name="plus" size={15} />创建群聊
          </button>
        </div>
      </div>
    </div>
  );
}

export default function ConversationsPage() {
  const [active, setActive] = useState('g-vocant');
  const [store, setStore] = useState<Record<string, Message[]>>(MESSAGES);
  const [showNew, setShowNew] = useState(false);
  const conv = CONVERSATIONS.find(c => c.id === active)!;
  const msgs = store[active] || [];

  const send = (text: string) => {
    setStore(s => ({ ...s, [active]: [...(s[active] || []), { from: 'me', time: '现在', blocks: [{ t: 'md', md: text }] }] }));
    const isG = conv.type === 'group';
    const mention = isG && text.match(/@([一-龥A-Za-z ]+)/);
    let responderId = isG ? conv.members![0] : conv.agent!;
    if (isG && mention) {
      const found = AGENTS.find(a => mention[1].trim().startsWith(a.name));
      if (found) responderId = found.id;
    }
    setStore(s => ({ ...s, [active]: [...(s[active] || []), { from: responderId, time: '', blocks: [{ t: 'typing' }], _temp: true }] }));
    setTimeout(() => {
      setStore(s => {
        const arr = (s[active] || []).filter(m => !m._temp);
        const reply = isG
          ? `收到 @管理员 的消息。我是${AGENT_MAP[responderId].name}，已结合群聊上下文与附件分析，稍后给出结构化结论。`
          : '已收到，我正在处理你的请求并会同步进展。';
        return { ...s, [active]: [...arr, { from: responderId, time: '现在', blocks: [{ t: 'md', md: reply }] }] };
      });
    }, 1600);
  };

  return (
    <>
      <ConvList active={active} onSelect={setActive} onNew={() => setShowNew(true)} />
      {conv
        ? <ChatView key={active} conv={conv} msgs={msgs} onSend={send} />
        : <div className="content"><div className="empty"><Icon name="chat" /><div>选择一个对话开始</div></div></div>}
      {showNew && <NewGroupModal onClose={() => setShowNew(false)} />}
    </>
  );
}
