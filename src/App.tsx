import React, { useState, useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import Icon from './components/Icon';
import { MeAvatar } from './components/Avatar';
import Dashboard from './pages/Dashboard';
import ConversationsPage from './pages/Conversations';
import AuditPage from './pages/Audit';
import SettingsPage from './pages/Settings';
import { getSystemHealth, type SystemHealth } from './services';

type Page = 'home' | 'chat' | 'audit' | 'settings';
type Theme = 'dark' | 'light';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
const win = () => getCurrentWindow();

// ---- Traffic light buttons ----
function TrafficLights() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!isTauri) return;
    win().isMaximized().then(setMaximized);
    let unlisten: (() => void) | undefined;
    win().onResized(() => win().isMaximized().then(setMaximized))
         .then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, []);

  // stopPropagation on mousedown prevents startDragging from firing
  const stopDrag = (e: React.MouseEvent) => e.stopPropagation();

  return (
    <div className="traffic">
      <button
        className="traffic-btn r"
        title="关闭"
        onMouseDown={stopDrag}
        onClick={() => isTauri && win().close()}
      >
        <Icon name="winClose" size={8} />
      </button>
      <button
        className="traffic-btn y"
        title="最小化"
        onMouseDown={stopDrag}
        onClick={() => isTauri && win().minimize()}
      >
        <Icon name="winMinimize" size={8} />
      </button>
      <button
        className={'traffic-btn g' + (maximized ? ' restore' : '')}
        title={maximized ? '还原' : '最大化'}
        onMouseDown={stopDrag}
        onClick={async () => {
          if (!isTauri) return;
          await win().toggleMaximize();
          setMaximized(await win().isMaximized());
        }}
      >
        <Icon name={maximized ? 'winRestore' : 'winMaximize'} size={9} />
      </button>
    </div>
  );
}

// ---- Titlebar drag ----
function handleTitlebarMouseDown(e: React.MouseEvent<HTMLDivElement>) {
  // Only drag from the titlebar itself, not from child interactive elements
  if ((e.target as HTMLElement).closest('button, a, input')) return;
  if (isTauri) win().startDragging();
}

// ---- Logo ----
// AutoForge mark: a stylized "A" whose crossbar is a forward-pointing pipeline arrow
// (the autonomous analysis→code→merge flow), capped by a small ember at the apex
// (the forge spark that the factory produces). Monoline, white on molten orange.
function ForgeLogo({ size = 38 }: { size?: number }) {
  return (
    <svg viewBox="0 0 38 38" width={size} height={size} fill="none">
      <rect width="38" height="38" rx="11" fill="url(#logo-bg)" />
      <defs>
        <linearGradient id="logo-bg" x1="0" y1="0" x2="38" y2="38" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#f5a623" />
          <stop offset="55%" stopColor="#e8772e" />
          <stop offset="100%" stopColor="#d45d1c" />
        </linearGradient>
      </defs>

      {/* 外圆 */}
      <circle cx="19" cy="19" r="15" stroke="#211a13" strokeWidth="1.5" fill="none" opacity="0.4"/>
      {/* 中圆 */}
      <circle cx="19" cy="19" r="9" stroke="#211a13" strokeWidth="1.5" fill="none" opacity="0.3"/>
      {/* 内圆 */}
      <circle cx="19" cy="19" r="4.5" stroke="#211a13" strokeWidth="1.5" fill="none" opacity="0.2"/>

      {/* 三横 */}
      <line x1="0" y1="9.5" x2="36" y2="9.5" stroke="#211a13" strokeWidth="1.2" opacity="0.2"/>
      <line x1="0" y1="19" x2="36" y2="19" stroke="#211a13" strokeWidth="1.2" opacity="0.2"/>
      <line x1="0" y1="28.5" x2="36" y2="28.5" stroke="#211a13" strokeWidth="1.2" opacity="0.2"/>

      {/* 三竖 */}
      <line x1="9.5" y1="0" x2="9.5" y2="36" stroke="#211a13" strokeWidth="1.2" opacity="0.2"/>
      <line x1="19" y1="0" x2="19" y2="36" stroke="#211a13" strokeWidth="1.2" opacity="0.2"/>
      <line x1="28.5" y1="0" x2="28.5" y2="36" stroke="#211a13" strokeWidth="1.2" opacity="0.2"/>
    </svg>
  );
}

const NAV: { id: Page; name: string; ic: string; badge?: number }[] = [
  { id: 'home',  name: '主页',     ic: 'home' },
  { id: 'chat',  name: '对话',     ic: 'chat',  badge: 3 },
  { id: 'audit', name: '功能审计', ic: 'audit', badge: 4 },
];

export default function App() {
  const [page,  setPage]  = useState<Page>('home');
  const [theme, setTheme] = useState<Theme>('dark');
  const [health, setHealth] = useState<SystemHealth | null>(null);
  const [lastEvent, setLastEvent] = useState('');

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme);
  }, [theme]);

  useEffect(() => {
    if (!isTauri) return;
    const refresh = () => getSystemHealth().then(setHealth).catch(() => setHealth(null));
    refresh();
    let unlisten: (() => void) | undefined;
    listen<Record<string, unknown>>('autoforge://event', e => {
      const type = typeof e.payload?.type === 'string' ? e.payload.type : 'event';
      setLastEvent(type);
      refresh();
    }).then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, []);

  const stageLabel = health
    ? health.stage === 'paused' ? '系统暂停'
      : health.stage === 'throttled' ? '单线程降速'
      : '流水线运行中'
    : '状态检测中';

  return (
    <div className="os-window">
      {/* Custom titlebar — onMouseDown triggers window drag */}
      <div className="os-titlebar" onMouseDown={handleTitlebarMouseDown}>
        <TrafficLights />
        <div className="tb-title">AUTO<b>FORGE</b> · 通用软件工厂</div>
        <div className="tb-right">
          <span className={'chip ' + (health?.stage === 'paused' ? 'red' : health?.stage === 'throttled' ? 'amber' : 'green')} style={{ padding: '3px 9px' }}>
            <span className={'dot ' + (health?.stage === 'paused' ? 'red' : 'green')} style={{ width: 6, height: 6, boxShadow: 'none' }} />
            {stageLabel}
            {health && <span style={{ marginLeft: 6, fontFamily: 'var(--font-mono)' }}>{health.active_slots}/{health.max_slots}</span>}
            {lastEvent && <span style={{ marginLeft: 6, color: 'var(--text-faint)' }}>{lastEvent}</span>}
          </span>
        </div>
      </div>

      <div className="os-body">
        <div className="rail">
          <div className="rail-logo" title="AutoForge">
            <ForgeLogo size={22} />
          </div>
          {NAV.map(n => (
            <button
              key={n.id}
              className={'rail-item' + (page === n.id ? ' active' : '')}
              onClick={() => setPage(n.id)}
              title={n.name}
            >
              <Icon name={n.ic} size={23} />
              {n.badge && page !== n.id && <span className="rail-badge">{n.badge}</span>}
            </button>
          ))}
          <div className="rail-spacer" />
          <button
            className="rail-item"
            title={theme === 'dark' ? '切换浅色' : '切换深色'}
            onClick={() => setTheme(t => t === 'dark' ? 'light' : 'dark')}
          >
            <Icon name={theme === 'dark' ? 'sun' : 'moon'} size={22} />
          </button>
          <button
            className={'rail-item' + (page === 'settings' ? ' active' : '')}
            onClick={() => setPage('settings')}
            title="设置"
          >
            <Icon name="settings" size={23} />
          </button>
          <div style={{ marginTop: 6 }}>
            <MeAvatar size={38} />
          </div>
        </div>

        {page === 'home'     && <Dashboard />}
        {page === 'chat'     && <ConversationsPage />}
        {page === 'audit'    && <AuditPage />}
        {page === 'settings' && <SettingsPage />}
      </div>
    </div>
  );
}
