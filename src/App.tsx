import React, { useState, useEffect } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import Icon from './components/Icon';
import { MeAvatar } from './components/Avatar';
import Dashboard from './pages/Dashboard';
import ConversationsPage from './pages/Conversations';
import AuditPage from './pages/Audit';
import SettingsPage from './pages/Settings';

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
      />
      <button
        className="traffic-btn y"
        title="最小化"
        onMouseDown={stopDrag}
        onClick={() => isTauri && win().minimize()}
      />
      <button
        className="traffic-btn g"
        title={maximized ? '还原' : '最大化'}
        onMouseDown={stopDrag}
        onClick={() => isTauri && win().toggleMaximize()}
      />
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
function ForgeLogo({ size = 22 }: { size?: number }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none"
         stroke="#2a1607" strokeWidth="2.1" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 20h10" />
      <path d="M14 14l6-6-3-3-6 6z" />
      <path d="M8 20l4-9 3 3-3 6" />
      <path d="M17 5l2-2" />
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

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme);
  }, [theme]);

  return (
    <div className="os-window">
      {/* Custom titlebar — onMouseDown triggers window drag */}
      <div className="os-titlebar" onMouseDown={handleTitlebarMouseDown}>
        <TrafficLights />
        <div className="tb-title">AUTO<b>FORGE</b> · 通用软件工厂</div>
        <div className="tb-right">
          <span className="chip green" style={{ padding: '3px 9px' }}>
            <span className="dot green" style={{ width: 6, height: 6, boxShadow: 'none' }} />
            流水线运行中
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
