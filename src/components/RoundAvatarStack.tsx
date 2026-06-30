import React, { useEffect, useRef, useState } from 'react';
import { Avatar, MeAvatar } from './Avatar';
import type { Agent } from '../services';

/**
 * 会议室左侧「对话轮次标尺 + 发言者头像堆叠」。
 *
 * 垂直居中悬浮在消息区左侧，自上而下：
 *   之前轮次的刻度横线 → 「我」按钮(当前轮) → 当前轮发言者头像(按发言序) → 之后轮次的刻度横线。
 *
 * 设计要点：
 * - 数据解耦：父级把每一轮的预览文本(`rounds[].text`)与当前段发言者(`agents`)算好传入，
 *   本组件不感知 `msgs`/`msgText`，只负责呈现与交互。
 * - 命中区连续：项间距由各按钮内边距提供、容器 `gap:0`，鼠标在整列内移动 hover 不丢。
 * - 零位移/零缩放：hover 不做位移或放大特效（只切换刻度/头像描边颜色与投影），鼠标稳定、选中精准不抖。
 * - hover 内容/名称气泡为实底面板，自带描边+阴影，无需虚化背景；离开留 100ms 意图缓冲防闪。
 */

export interface RoundInfo {
  /** 该轮起始「我的发言」消息 id（点击跳转目标） */
  id: string;
  /** 该轮我的发言文本（hover 预览，已压缩/裁剪由父级决定，这里原样展示） */
  text: string;
}

interface Props {
  /** 全部轮次（=「我的发言」），按时间先后。 */
  rounds: RoundInfo[];
  /** 当前滚动位置所在轮的下标；-1 表示位于首条发言之前。 */
  currentRoundIndex: number;
  /** 当前段发言者（已解析为 Agent，按发言先后）。 */
  agents: Agent[];
  /** 当前正在阅读的气泡作者 id → 彩色高亮其头像边框。 */
  currentAgentId: string | null;
  /** 跳到某条消息（轮次起始 / 我的发言）。 */
  onJump: (messageId: string) => void;
  /** 跳到该 agent 在「当前所示轮次」内的发言。 */
  onJumpAgent: (agentId: string) => void;
}

const ME = '__me__';
const PANEL: React.CSSProperties = {
  width: 'max-content', maxWidth: 280, maxHeight: 200, overflowY: 'auto',
  background: 'var(--bg-2)', border: '1px solid var(--border-strong)', borderLeft: '2px solid var(--ember)',
  borderRadius: 10, padding: '8px 12px', boxShadow: 'var(--shadow)', textAlign: 'left',
  fontSize: 'var(--text-control)', color: 'var(--text)', lineHeight: 'var(--leading-normal)',
  whiteSpace: 'pre-wrap', wordBreak: 'break-word',
};
const PILL: React.CSSProperties = {
  fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)',
  background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 8,
  padding: '3px 9px', whiteSpace: 'nowrap', boxShadow: 'var(--shadow)',
};
// 头像外环：当前作者 ember 彩色，其余同宽灰色；hover 仅加大投影（不影响布局）。
const ring = (current: boolean, hovered: boolean): string =>
  `0 0 0 2px ${current ? 'var(--ember)' : 'var(--border-strong)'}, ${hovered ? 'var(--shadow)' : 'var(--shadow-sm)'}`;

const btnBase: React.CSSProperties = {
  pointerEvents: 'auto', position: 'relative', border: 0, background: 'transparent',
  cursor: 'pointer', display: 'flex', gap: 7,
};

// 头像统一尺寸（精致小号）。
const AV = 26;

export default function RoundAvatarStack({
  rounds, currentRoundIndex, agents, currentAgentId, onJump, onJumpAgent,
}: Props) {
  const [hover, setHover] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const enter = (id: string) => {
    if (timer.current) { clearTimeout(timer.current); timer.current = null; }
    setHover(id);
  };
  const leave = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => { setHover(null); timer.current = null; }, 100);
  };
  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); }, []);

  if (rounds.length === 0 && agents.length === 0) return null;

  const prevRounds = currentRoundIndex > 0 ? rounds.slice(0, currentRoundIndex) : [];
  const nextRounds = rounds.slice(currentRoundIndex + 1); // currentRoundIndex=-1 → 全部
  const currentRound = currentRoundIndex >= 0 ? rounds[currentRoundIndex] : undefined;

  // 一根「轮次刻度横线」：hover 显示那一轮我的发言、点击跳到那一轮。
  const tick = (r: RoundInfo) => {
    const h = hover === r.id;
    return (
      <button
        key={r.id} onMouseEnter={() => enter(r.id)} onMouseLeave={leave} onClick={() => onJump(r.id)}
        style={{ ...btnBase, alignItems: 'center', padding: '5px 0', zIndex: h ? 60 : 1 }}
      >
        <span style={{ display: 'flex', width: AV, justifyContent: 'center' }}>
          <span style={{
            width: 16, height: 2, borderRadius: 2,
            background: h ? 'var(--ember)' : 'var(--border-strong)',
            transition: 'background .14s ease',
          }} />
        </span>
        {h && <div className="scroll" style={PANEL}>{r.text || '（无文本内容）'}</div>}
      </button>
    );
  };

  return (
    <div style={{
      position: 'absolute',
      // 锚定左上角：紧贴 chat-head(60px) 下方一点；窗口够宽时退进左侧留白槽、与气泡拉开间距
      //（窄屏自动收回贴左、不越界）。不再垂直居中——精致地待在角落，不占据页面中部。
      left: 'max(12px, calc((100% - var(--thread-max)) / 2 - 42px))',
      top: 74,
      zIndex: 6, display: 'flex', flexDirection: 'column', alignItems: 'flex-start',
      gap: 0, pointerEvents: 'none',
    }}>
      {prevRounds.map(tick)}

      {currentRound && (() => {
        const h = hover === ME;
        return (
          <button
            onMouseEnter={() => enter(ME)} onMouseLeave={leave} onClick={() => onJump(currentRound.id)}
            style={{ ...btnBase, alignItems: 'flex-start', padding: '4px 0', zIndex: h ? 60 : 2 }}
          >
            <span style={{ display: 'block', borderRadius: 9, lineHeight: 0, boxShadow: ring(false, h), transition: 'box-shadow .16s ease' }}>
              <MeAvatar size={AV} />
            </span>
            {h && <div className="scroll" style={PANEL}>{currentRound.text || '（无文本内容）'}</div>}
          </button>
        );
      })()}

      {agents.map(a => {
        const h = hover === a.id;
        const cur = currentAgentId === a.id;
        return (
          <button
            key={a.id} onMouseEnter={() => enter(a.id)} onMouseLeave={leave} onClick={() => onJumpAgent(a.id)}
            style={{ ...btnBase, alignItems: 'center', padding: '4px 0', zIndex: h ? 60 : cur ? 50 : 1 }}
          >
            <span style={{ display: 'block', borderRadius: 9, lineHeight: 0, boxShadow: ring(cur, h), transition: 'box-shadow .16s ease' }}>
              <Avatar agent={a} size={AV} />
            </span>
            {h && <span style={{ ...PILL, color: a.color || 'var(--text)' }}>{a.name}</span>}
          </button>
        );
      })}

      {nextRounds.map(tick)}
    </div>
  );
}
