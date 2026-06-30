import React, { useEffect, useRef, useState } from 'react';
import { Avatar } from './Avatar';
import type { Agent } from '../services';

/**
 * 会议室左侧「对话轮次刻度尺 + 悬浮目录」。
 *
 * 默认态：只悬浮一条极简的竖向刻度尺（每轮一根刻度），清爽不抢视线。
 * hover 刻度尺 → 右侧弹出一个目录面板（TOC）：
 *   - 每一项标题 = 那一轮「我的发言」；
 *   - 标题下挂该轮**所有 agent 回复**的子列表，默认收起；
 *   - hover 某条标题 → 展开它的子列表，点子项快速跳到该 agent 的那条发言。
 *
 * 设计要点：
 * - 数据解耦：父级把每一轮的预览文本与该轮回复(`rounds[].replies`)算好传入，
 *   本组件不感知 `msgs`/`msgText`，只负责呈现与交互。
 * - 零位移/零缩放：hover 不做位移或放大特效，靠颜色/投影/展开切换，鼠标稳定、选中精准不抖。
 * - 刻度尺与面板包在同一 pointer-events 热区内（含中间 gap），横移不经过死区；
 *   再叠 320ms 离开意图缓冲，从刻度尺移到面板、或在面板内移动都不闪、来得及进入。
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
  const [open, setOpen] = useState(false);          // 面板是否展开（hover 刻度尺/面板）
  const [hoverRound, setHoverRound] = useState<string | null>(null); // 当前展开子列表的轮 id
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const enter = () => { if (timer.current) { clearTimeout(timer.current); timer.current = null; } setOpen(true); };
  const leave = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => { setOpen(false); setHoverRound(null); timer.current = null; }, 320);
  };
  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); }, []);

  if (rounds.length === 0) return null;

  return (
    <div
      style={{
        position: 'absolute',
        // 锚定左上角：紧贴 chat-head 下方一点；窗口够宽时退进左侧留白槽、与气泡拉开间距。
        left: 'max(12px, calc((100% - var(--thread-max)) / 2 - 42px))',
        top: 74, zIndex: 6,
        display: 'flex', alignItems: 'flex-start',
        pointerEvents: 'none',
      }}
    >
      {/* 刻度尺 + 面板包在同一悬浮区：gap 也算热区，鼠标从刻度尺横移到面板不经过死区、不闪。 */}
      <div
        onMouseEnter={enter} onMouseLeave={leave}
        style={{
          pointerEvents: 'auto', display: 'flex', alignItems: 'flex-start', gap: 10,
        }}
      >
      {/* 刻度尺：默认唯一可见物。每轮一根刻度，当前轮 ember 高亮。 */}
      <div
        style={{
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
          const hot = open && (hoverRound === r.id || (hoverRound === null && cur));
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
      </div>

      {/* hover 目录面板：标题=我的发言，子列表=该轮所有 agent 回复（默认收起，hover 标题展开）。 */}
      {open && (
        <div className="scroll" style={PANEL}>
          <div style={KICKER}>对话目录 · {rounds.length} 轮</div>
          {rounds.map((r, i) => {
            const cur = i === currentRoundIndex;
            const expanded = hoverRound === r.id;
            return (
              <div
                key={r.id}
                onMouseEnter={() => setHoverRound(r.id)}
                style={{ display: 'flex', flexDirection: 'column' }}
              >
                {/* 轮标题：我的发言 */}
                <button
                  onClick={() => onJump(r.id)}
                  style={{
                    ...rowBtn,
                    background: cur ? 'var(--ember-tint)' : expanded ? 'var(--surface-hover)' : 'transparent',
                  }}
                >
                  <span style={{
                    width: 6, height: 6, borderRadius: 99, flexShrink: 0,
                    background: cur ? 'var(--ember)' : 'var(--border-strong)',
                  }} />
                  <span style={clamp1}>{r.text || '（无文本内容）'}</span>
                  {r.replies.length > 0 && (
                    <span style={{
                      fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)',
                      color: 'var(--text-faint)', flexShrink: 0,
                    }}>{r.replies.length}</span>
                  )}
                </button>

                {/* 子列表：该轮所有 agent 回复，hover 标题才展开。 */}
                {expanded && r.replies.length > 0 && (
                  <div style={{
                    display: 'flex', flexDirection: 'column', gap: 1,
                    margin: '1px 0 3px', paddingLeft: 10,
                    borderLeft: '1px solid var(--border)', marginLeft: 12,
                  }}>
                    {r.replies.map(rep => (
                      <button
                        key={rep.msgId}
                        onClick={() => onJump(rep.msgId)}
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
    </div>
  );
}
