import React, { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import Icon from './components/Icon';
import { MeAvatar } from './components/Avatar';
import Dashboard from './pages/Dashboard';
import ConversationsPage from './pages/Conversations';
import AuditPage from './pages/Audit';
import ProjectsPage from './pages/Projects';
import SettingsPage from './pages/Settings';
import { getSystemHealth, checkClaudeAuth, getBadgeCounts, type SystemHealth } from './services';
import { THEME_STORAGE_KEY, oppositeMode, parseTheme, themeIdOf, type ThemeSelection } from './theme';

type Page = 'home' | 'chat' | 'projects' | 'audit' | 'settings';

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

// ---- Titlebar drag / double-click maximize ----
// Double-click is detected at mousedown level because startDragging() captures
// the mouse pointer on Linux/WebKitGTK, preventing the dblclick event from
// ever reaching the webview.
let lastTitlebarDown = 0;

function handleTitlebarMouseDown(e: React.MouseEvent<HTMLDivElement>) {
  if ((e.target as HTMLElement).closest('button, a, input')) return;
  if (!isTauri) return;
  const now = Date.now();
  if (now - lastTitlebarDown < 400) {
    lastTitlebarDown = 0;
    win().toggleMaximize();
    return;
  }
  lastTitlebarDown = now;
  win().startDragging();
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
      <circle cx="19" cy="19" r="15" stroke="#472811" strokeWidth="1.3" fill="none" opacity="0.3"/>
      {/* 中圆 */}
      <circle cx="19" cy="19" r="9" stroke="#472811" strokeWidth="1.3" fill="none" opacity="0.2"/>
      {/* 内圆 */}
      <circle cx="19" cy="19" r="4.5" stroke="#472811" strokeWidth="1.3" fill="none" opacity="0.1"/>

      {/* 三横 */}
      <line x1="0" y1="9.5" x2="38" y2="9.5" stroke="#472811" strokeWidth="0.8" opacity="0.2"/>
      <line x1="0" y1="19" x2="38" y2="19" stroke="#472811" strokeWidth="0.8" opacity="0.2"/>
      <line x1="0" y1="28.5" x2="38" y2="28.5" stroke="#472811" strokeWidth="0.8" opacity="0.2"/>

      {/* 三竖 */}
      <line x1="9.5" y1="0" x2="9.5" y2="38" stroke="#472811" strokeWidth="0.8" opacity="0.2"/>
      <line x1="19" y1="0" x2="19" y2="38" stroke="#472811" strokeWidth="0.8" opacity="0.2"/>
      <line x1="28.5" y1="0" x2="28.5" y2="38" stroke="#472811" strokeWidth="0.8" opacity="0.2"/>

      {/* 大写 A：左侧偏长，右侧偏短，复仇者风格 */}
      <g stroke="#5d3617ff" stroke-width="4" opacity="1" fill="none" stroke-linecap="round" stroke-linejoin="round">
        <line x1="21" y1="6" x2="11" y2="29.5" />
        <line x1="21" y1="6" x2="25" y2="23" />
        <line x1="15" y1="22" x2="22" y2="20" />
      </g>  
    </svg>
  );
}

const NAV: { id: Page; name: string; ic: string }[] = [
  { id: 'home',     name: '主页',     ic: 'home' },
  { id: 'chat',     name: '会议室',   ic: 'chat' },
  { id: 'projects', name: '项目管理', ic: 'box' },
  { id: 'audit',    name: '功能审计', ic: 'audit' },
];

export default function App() {
  const [page,  setPage]  = useState<Page>(() => {
    const saved = sessionStorage.getItem('AutoForge:page') as Page | null;
    return saved && (['home', 'chat', 'projects', 'audit', 'settings'] as string[]).includes(saved) ? saved : 'home';
  });
  const [theme, setTheme] = useState<ThemeSelection>(() => parseTheme(localStorage.getItem(THEME_STORAGE_KEY)));
  const [health, setHealth] = useState<SystemHealth | null>(null);
  const [badges, setBadges] = useState({ chat: 0, audit: 0 });

  const badgeRefreshInFlight = useRef(false);
  const refreshBadges = useCallback(async () => {
    if (badgeRefreshInFlight.current) return;
    badgeRefreshInFlight.current = true;
    try {
      const counts = await getBadgeCounts();
      setBadges({ chat: counts.chat_unread, audit: counts.audit_pending });
    } catch {
      // ignore — badges stay stale rather than crashing
    } finally {
      badgeRefreshInFlight.current = false;
    }
  }, []);

  // Moved out of the Tauri event effect so the useRef guard survives
  // React StrictMode's double-invocation (local variables inside the effect
  // get two separate copies and can't block concurrent calls).
  const healthInFlight = useRef(false);
  const refreshHealth = useCallback(() => {
    if (healthInFlight.current) return;
    healthInFlight.current = true;
    getSystemHealth()
      .then(h => { setHealth(h); })
      .catch(() => setHealth(null))
      .finally(() => { healthInFlight.current = false; });
  }, []);

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme.mode);
    document.documentElement.setAttribute('data-palette', theme.palette);
    localStorage.setItem(THEME_STORAGE_KEY, themeIdOf(theme));
  }, [theme]);

  // Auth check intentionally removed: spawning the claude Electron subprocess
  // at any point while WebKitGTK is active delivers SIGTRAP to our process,
  // triggering a NeedDebuggerBreak trap that permanently freezes IPC.
  // Auth errors surface naturally when pipeline tasks fail to run.

  useEffect(() => {
    refreshBadges();
    const onCustom = () => refreshBadges();
    window.addEventListener('AutoForge:badges-refresh', onCustom);
    return () => window.removeEventListener('AutoForge:badges-refresh', onCustom);
  }, [refreshBadges]);

  // Single consolidated listener for all AutoForge://event traffic.
  // Three separate listen() calls were previously registered here — each event
  // fired all three handlers simultaneously, causing 5+ concurrent IPC calls
  // (including a Rust check_auth() that spawns 2 subprocesses every time).
  useEffect(() => {
    if (!isTauri) return;

    const notify = (title: string, body: string) => {
      try {
        if (typeof Notification === 'undefined') return;
        if (Notification.permission === 'granted') {
          new Notification(title, { body });
        } else if (Notification.permission !== 'denied') {
          Notification.requestPermission().then(p => {
            if (p === 'granted') new Notification(title, { body });
          });
        }
      } catch { /* notifications unavailable — ignore */ }
    };

    // Debounce heavy refresh calls: collapse bursts within 500 ms into one call.
    let badgeTimer: ReturnType<typeof setTimeout> | null = null;
    let healthTimer: ReturnType<typeof setTimeout> | null = null;
    const debouncedBadges = () => {
      if (badgeTimer) clearTimeout(badgeTimer);
      badgeTimer = setTimeout(() => { badgeTimer = null; refreshBadges(); }, 500);
    };
    const debouncedHealth = () => {
      if (healthTimer) clearTimeout(healthTimer);
      healthTimer = setTimeout(() => { healthTimer = null; refreshHealth(); }, 500);
    };

    let unlisten: (() => void) | undefined;
    listen<Record<string, unknown>>('AutoForge://event', e => {
      const ev = e.payload as {
        type?: string; issue_title?: string; stage?: number;
        cr_id?: string; iteration?: number; status?: string; summary?: string;
      };

      // Debounced IPC refreshes.
      debouncedBadges();
      debouncedHealth();

      // Desktop notifications (only for actionable events).
      switch (ev?.type) {
        case 'review_needed':
          notify(`需要审核 · 节点 ${ev.stage ?? '?'}`, ev.issue_title ?? '有新的待审核项');
          break;
        case 'iteration_warning':
          notify('迭代次数告警', `${ev.cr_id ?? ''} 已迭代 ${ev.iteration ?? ''} 轮，建议人工介入`);
          break;
        case 'cr_merged':
          notify('已合并到 dev', ev.cr_id ?? '');
          break;
        case 'test_completed':
          notify(ev.status === 'passed' ? '测试通过' : '测试失败', ev.summary ?? '');
          break;
        default:
          break;
      }
    }).then(fn => { unlisten = fn; });

    // Delay the initial health check by 2 s so that the critical startup IPC
    // calls (list_conversations, mark_conversation_read, etc.) complete before
    // we spawn the `claude auth status` subprocess.  The subprocess exit triggers
    // a NeedDebuggerBreak trap in WebKitGTK that disrupts the GTK event loop;
    // delaying it past the sensitive startup window avoids the freeze.
    const startupHealthTimer = setTimeout(() => refreshHealth(), 2000);

    return () => {
      clearTimeout(startupHealthTimer);
      if (badgeTimer) clearTimeout(badgeTimer);
      if (healthTimer) clearTimeout(healthTimer);
      unlisten?.();
    };
  }, [refreshBadges, refreshHealth]);

  const stageLabel = health
    ? health.stage === 'paused' ? '系统暂停'
      : health.stage === 'throttled' ? '单线程降速'
      : '流水线运行中'
    : '状态检测中';
  const navBadge = (id: Page) => id === 'chat' ? badges.chat : id === 'audit' ? badges.audit : 0;

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
            {health && <span style={{ marginLeft: 6, fontFamily: 'var(--font-mono)' }}>{health.active_slots}/{health.total_slot_capacity}</span>}
          </span>
        </div>
      </div>

      <div className="os-body">
        <div className="rail">
          <div className="rail-logo" title="AutoForge">
            <ForgeLogo size={22} />
          </div>
          {NAV.map(n => {
            const badge = navBadge(n.id);
            return (
            <button
              key={n.id}
              className={'rail-item' + (page === n.id ? ' active' : '')}
              onClick={() => { setPage(n.id); sessionStorage.setItem('AutoForge:page', n.id); }}
              title={n.name}
            >
              <Icon name={n.ic} size={23} />
              {badge > 0 && page !== n.id && <span className="rail-badge">{badge}</span>}
            </button>
            );
          })}
          <div className="rail-spacer" />
          <button
            className="rail-item"
            title={theme.mode === 'dark' ? '切换浅色' : '切换深色'}
            onClick={() => setTheme(t => ({ ...t, mode: oppositeMode(t.mode) }))}
          >
            <Icon name={theme.mode === 'dark' ? 'sun' : 'moon'} size={22} />
          </button>
          <button
            className={'rail-item' + (page === 'settings' ? ' active' : '')}
            onClick={() => { setPage('settings'); sessionStorage.setItem('AutoForge:page', 'settings'); }}
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
        {page === 'projects' && <ProjectsPage />}
        {page === 'audit'    && <AuditPage />}
        {page === 'settings' && <SettingsPage theme={theme} onThemeChange={setTheme} />}
      </div>
    </div>
  );
}
