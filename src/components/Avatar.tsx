import React from 'react';
import { AGENT_MAP, type Agent as MockAgent } from '../data/mock';
import { PixelAvatar } from './PixelAvatar';
import { useOperator } from '../operator';

type AgentLike = MockAgent | {
  id?: string;
  color: string;
  initial: string;
};

interface AvatarProps {
  agent: string | AgentLike;
  size?: number;
  status?: 'online' | 'busy' | 'offline';
}

export function Avatar({ agent, size = 40, status }: AvatarProps) {
  const a = typeof agent === 'string' ? AGENT_MAP[agent] : agent;
  const color = a ? a.color : '#e8772e';
  const seed = a && 'id' in a && a.id ? a.id : (typeof agent === 'string' ? agent : null);

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
