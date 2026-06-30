import React, { useEffect, useRef, useState } from 'react';
import { Avatar } from './Avatar';
import Icon from './Icon';
import type { Agent } from '../services';

/**
 * 会议室左侧「对话轮次刻度尺 + 目录」。
 *
 * 默认态：只悬浮一条极简的竖向刻度尺（每轮一根刻度），清爽不抢视线。
 * 点击刻度尺 → 右侧弹出一个目录面板（TOC）；再次点击刻度尺 / 点击面板外 / Esc 收起。
 *   - 每一项标题 = 那一轮「我的发言」；
 *   - 标题下挂该轮**所有 agent 回复**的子列表，默认收起。
 *
 * 交互全部走点击（无 hover 触发），且每行**区分两个点击事件**：
 *   1. 点击标题文字 → 跳转到该轮发言；
 *   2. 点击右侧 caret（角标）→ 展开/收起该轮的 agent 回复子列表；
 *   子项点击 → 跳转到该 agent 的那条发言。
 *
 * 设计要点：
 * - 数据解耦：父级把每一轮的预览文本与该轮回复(`rounds[].replies`)算好传入，
 *   本组件不感知 `msgs`/`msgText`，只负责呈现与交互。
 * - 零位移/零缩放：不做位移或放大特效，靠颜色/投影/展开切换，鼠标稳定、选中精准不抖。
 */

/** 一轮里某个 agent 的回复（子列表项）。 */
export interface ReplyNav {
  /** 该 agent 在这一轮的发言消息 id（点击跳转目标）。 */
  msgId: string;
  /** 发言者（用于头像 / 名称 / 颜色）。 */
  agent: Agent;
  /** 该条回复的预览文本（已由父级裁剪/压缩，这里原样展示）。 */
  text: string;
}

/** 一轮对话（=「我的发言」+ 该轮所有 agent 回复）。 */
export interface RoundNav {
  /** 该轮起始「我的发言」消息 id（点击跳转目标）。 */
  id: string;
  /** 该轮我的发言预览文本。 */
  text: string;
  /** 该轮所有 agent 回复，按发言先后。 */
  replies: ReplyNav[];
}

// 兼容旧引用名。
export type RoundInfo = RoundNav;

interface Props {
  /** 全部轮次（=「我的发言」），按时间先后。 */
  rounds: RoundNav[];
  /** 当前滚动位置所在轮的下标；-1 表示位于首条发言之前。 */
  currentRoundIndex: number;
  /** 跳到某条消息（我的发言 / 某 agent 回复）。 */
  onJump: (messageId: string) => void;
}

const AV = 18;

const PANEL: React.CSSProperties = {
  width: 280, maxHeight: '60vh', overflowY: 'auto',
  background: 'var(--bg-2)', border: '1px solid var(--border-strong)',
  borderRadius: 12, padding: 6, boxShadow: 'var(--shadow-lg)',
  display: 'flex', flexDirection: 'column', gap: 2,
};

const KICKER: React.CSSProperties = {
  fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', letterSpacing: '.14em',
  textTransform: 'uppercase', color: 'var(--text-faint)', padding: '4px 8px 6px',
};

const rowBtn: React.CSSProperties = {
  width: '100%', textAlign: 'left', border: 0, background: 'transparent',
  cursor: 'pointer', borderRadius: 9, padding: '7px 9px',
  display: 'flex', alignItems: 'center', gap: 8,
  fontSize: 'var(--text-control)', color: 'var(--text)',
  transition: 'background .12s ease',
};

const clamp1: React.CSSProperties = {
  display: '-webkit-box', WebkitLineClamp: 1, WebkitBoxOrient: 'vertical',
  overflow: 'hidden', wordBreak: 'break-word', flex: 1, minWidth: 0,
};
const clamp2: React.CSSProperties = { ...clamp1, WebkitLineClamp: 2 };

export default function RoundAvatarStack({ rounds, currentRoundIndex, onJump }: Props) {
  const [open, setOpen] = useState(false);                       // 面板是否展开（点击刻度尺切换）
  const [expanded, setExpanded] = useState<Set<string>>(new Set()); // 已展开子列表的轮 id 集合
  const rootRef = useRef<HTMLDivElement | null>(null);

  const toggleExpand = (id: string) => setExpanded(prev => {
    const next = new Set(prev);
    next.has(id) ? next.delete(id) : next.add(id);
    return next;
  });

  // 跳转后收起整个目录：面板浮在聊天流上，跳完即让位给正文。
  const jump = (id: string) => { onJump(id); setOpen(false); setExpanded(new Set()); };

  // 点击面板外 / Esc → 收起整个目录（点击式弹层的标准收起途径）。
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false); setExpanded(new Set());
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { setOpen(false); setExpanded(new Set()); }
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => { document.removeEventListener('mousedown', onDown); document.removeEventListener('keydown', onKey); };
  }, [open]);

  if (rounds.length === 0) return null;

  return (
    <div
      ref={rootRef}
      style={{
        position: 'absolute',
        // 锚定左上角：紧贴 chat-head 下方一点；窗口够宽时退进左侧留白槽、与气泡拉开间距。
        left: 'max(12px, calc((100% - var(--thread-max)) / 2 - 42px))',
        top: 74, zIndex: 6,
        display: 'flex', alignItems: 'flex-start', gap: 10,
        pointerEvents: 'none',
      }}
    >
      {/* 刻度尺：默认唯一可见物。点击切换目录面板开/关。 */}
      <button
        type="button"
        onClick={() => { setOpen(o => !o); if (open) setExpanded(new Set()); }}
        aria-label={open ? '收起对话目录' : '展开对话目录'}
        style={{
          pointerEvents: 'auto', border: 0, appearance: 'none',
          display: 'flex', flexDirection: 'column',
          gap: 7, padding: '5px 7px', cursor: 'pointer',
          borderRadius: 10,
          background: open ? 'var(--bg-2)' : 'transparent',
          boxShadow: open ? 'var(--shadow-sm)' : 'none',
          transition: 'background .16s ease, box-shadow .16s ease',
        }}
      >
        {rounds.map((r, i) => {
          const cur = i === currentRoundIndex;
          const hot = open && (expanded.has(r.id) || cur);
          return (
            <span key={r.id} style={{ display: 'flex', width: 20, justifyContent: 'center' }}>
              <span style={{
                width: cur ? 20 : 14, height: 3, borderRadius: 3,
                background: cur || hot ? 'var(--ember)' : 'var(--text-faint)',
                opacity: cur || hot ? 1 : (open ? 0.9 : 0.7),
                transition: 'background .14s ease, width .14s ease, opacity .14s ease',
              }} />
            </span>
          );
        })}
      </button>

      {/* 目录面板：标题=我的发言（点击跳转），caret=展开/收起该轮回复子列表。 */}
      {open && (
        <div className="scroll" style={{ ...PANEL, pointerEvents: 'auto' }}>
          <div style={KICKER}>对话目录 · {rounds.length} 轮</div>
          {rounds.map((r, i) => {
            const cur = i === currentRoundIndex;
            const isExp = expanded.has(r.id);
            const hasReplies = r.replies.length > 0;
            return (
              <div
                key={r.id}
                style={{ display: 'flex', flexDirection: 'column' }}
              >
                {/* 轮标题行：左=跳转按钮，右=展开/收起 caret（两个独立点击事件）。 */}
                <div
                  style={{
                    display: 'flex', alignItems: 'stretch', borderRadius: 9, overflow: 'hidden',
                    background: cur ? 'var(--ember-tint)' : isExp ? 'var(--surface-hover)' : 'transparent',
                  }}
                >
                  {/* 点击 1：跳转到该轮我的发言 */}
                  <button
                    onClick={() => jump(r.id)}
                    title="跳转到这轮发言"
                    style={{ ...rowBtn, flex: 1, minWidth: 0, background: 'transparent', borderRadius: 0 }}
                    onMouseEnter={e => { if (!cur) e.currentTarget.style.background = 'var(--surface-hover)'; }}
                    onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; }}
                  >
                    <span style={{
                      width: 6, height: 6, borderRadius: 99, flexShrink: 0,
                      background: cur ? 'var(--ember)' : 'var(--border-strong)',
                    }} />
                    <span style={clamp1}>{r.text || '（无文本内容）'}</span>
                  </button>

                  {/* 点击 2：展开/收起该轮 agent 回复子列表 */}
                  {hasReplies && (
                    <button
                      onClick={() => toggleExpand(r.id)}
                      title={isExp ? '收起本轮回复' : `展开本轮 ${r.replies.length} 条回复`}
                      aria-expanded={isExp}
                      style={{
                        border: 0, background: 'transparent', cursor: 'pointer',
                        display: 'flex', alignItems: 'center', gap: 4,
                        padding: '0 9px', flexShrink: 0, color: 'var(--text-faint)',
                        transition: 'background .12s ease',
                      }}
                      onMouseEnter={e => { e.currentTarget.style.background = 'var(--surface-hover)'; }}
                      onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; }}
                    >
                      <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)' }}>
                        {r.replies.length}
                      </span>
                      <Icon name="chevron" size={14} style={{
                        transform: isExp ? 'rotate(180deg)' : 'none',
                        transition: 'transform .16s ease',
                      }} />
                    </button>
                  )}
                </div>

                {/* 子列表：该轮所有 agent 回复，点击 caret 展开。 */}
                {isExp && hasReplies && (
                  <div style={{
                    display: 'flex', flexDirection: 'column', gap: 1,
                    margin: '1px 0 3px', paddingLeft: 10,
                    borderLeft: '1px solid var(--border)', marginLeft: 12,
                  }}>
                    {r.replies.map(rep => (
                      <button
                        key={rep.msgId}
                        onClick={() => jump(rep.msgId)}
                        style={{ ...rowBtn, padding: '5px 8px', alignItems: 'flex-start' }}
                        onMouseEnter={e => { e.currentTarget.style.background = 'var(--surface-hover)'; }}
                        onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; }}
                      >
                        <span style={{ flexShrink: 0, marginTop: 1 }}><Avatar agent={rep.agent} size={AV} /></span>
                        <span style={{ display: 'flex', flexDirection: 'column', gap: 1, minWidth: 0, flex: 1 }}>
                          <span style={{
                            fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)',
                            color: rep.agent.color || 'var(--text-2)', whiteSpace: 'nowrap',
                            overflow: 'hidden', textOverflow: 'ellipsis',
                          }}>{rep.agent.name}</span>
                          <span style={{
                            ...clamp2, fontSize: 'var(--text-caption)', color: 'var(--text-3)',
                            lineHeight: 'var(--leading-normal)',
                          }}>{rep.text || '（无文本内容）'}</span>
                        </span>
                      </button>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
