import React from 'react';
import { useAgents, getAgentMap, type Agent } from '../agents-store';
import { PixelAvatar } from './PixelAvatar';
import { useOperator } from '../operator';

type AgentLike = Agent | {
  id?: string;
  color: string;
  initial: string;
};

interface AvatarProps {
  agent: string | AgentLike;
  size?: number;
  status?: 'online' | 'busy' | 'offline';
}

// 纯渲染：拿到已解析的 agent 与回退 seed 即可，不订阅任何 store。
// 绝大多数调用方直接传 Agent 对象，走这里，避免给每个头像挂无谓订阅与重渲染。
function AvatarView({ a, fallbackSeed, size, status }: {
  a?: AgentLike;
  fallbackSeed: string | null;
  size: number;
  status?: 'online' | 'busy' | 'offline';
}) {
  const color = a ? a.color : '#e8772e';
  const seed = a && 'id' in a && a.id ? a.id : fallbackSeed;

  return (
    <div className="av" style={{ width: size, height: size, borderRadius: size * 0.32 }}>
      {seed
        ? <PixelAvatar seed={seed} size={size} />
        : (
          <div
            style={{
              width: size,
              height: size,
              background: color,
              fontSize: size * 0.4,
              borderRadius: size * 0.32,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
            }}
          >
            {a ? a.initial : '?'}
          </div>
        )
      }
      {status && (
        <span
          className="av-status"
          style={{
            background:
              status === 'online'
                ? 'var(--green)'
                : status === 'busy'
                ? 'var(--amber)'
                : 'var(--text-faint)',
          }}
        />
      )}
    </div>
  );
}

// 仅 string id 形态订阅 DB Agent store 并查表（取代旧 mock AGENT_MAP）。
function AvatarById({ id, size, status }: {
  id: string; size: number; status?: 'online' | 'busy' | 'offline';
}) {
  useAgents();
  return <AvatarView a={getAgentMap()[id]} fallbackSeed={id} size={size} status={status} />;
}

export function Avatar({ agent, size = 40, status }: AvatarProps) {
  return typeof agent === 'string'
    ? <AvatarById id={agent} size={size} status={status} />
    : <AvatarView a={agent} fallbackSeed={agent.id ?? null} size={size} status={status} />;
}

export function MeAvatar({ size = 40 }: { size?: number }) {
  const op = useOperator();
  const accent = op.accent_color.trim();
  return (
    <div
      className="av"
      style={{
        width: size,
        height: size,
        fontSize: size * (op.avatar.length > 1 ? 0.34 : 0.4),
        borderRadius: size * 0.32,
        background: accent || 'var(--me-avatar-bg)',
        color: accent ? '#fff' : 'var(--me-avatar-color)',
        border: '1px solid var(--border-strong)',
      }}
    >
      {op.avatar || op.display_name.slice(0, 1) || '我'}
    </div>
  );
}
