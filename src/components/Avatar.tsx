import React from 'react';
import { AGENT_MAP, type Agent } from '../data/mock';

interface AvatarProps {
  agent: string | Agent;
  size?: number;
  status?: 'online' | 'busy' | 'offline';
}

export function Avatar({ agent, size = 40, status }: AvatarProps) {
  const a = typeof agent === 'string' ? AGENT_MAP[agent] : agent;
  const bg = a ? a.color : '#e8772e';
  const txt = a ? a.initial : '?';
  return (
    <div
      className="av"
      style={{
        width: size,
        height: size,
        background: bg,
        fontSize: size * 0.4,
        borderRadius: size * 0.32,
      }}
    >
      {txt}
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
  return (
    <div
      className="av"
      style={{
        width: size,
        height: size,
        fontSize: size * 0.4,
        borderRadius: size * 0.32,
        background: 'linear-gradient(135deg,#3a2c1c,#211a13)',
        color: 'var(--ember-soft)',
        border: '1px solid var(--border-strong)',
      }}
    >
      管
    </div>
  );
}
