import React, { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import Icon from './components/Icon';
import logoUrl from './assets/logo.png';
import { MeAvatar } from './components/Avatar';
import Dashboard from './pages/Dashboard';
import ConversationsPage from './pages/Conversations';
import AuditPage from './pages/Audit';
import ProjectsPage from './pages/Projects';
import DeliveryPage from './pages/Delivery';
import SettingsPage from './pages/Settings';
import { getSystemHealth, checkClaudeAuth, getBadgeCounts, type SystemHealth } from './services';
import { THEME_STORAGE_KEY, RAIL_STORAGE_KEY, applyRailMode, oppositeMode, parseRailMode, parseTheme, themeIdOf, type ThemeSelection } from './theme';

type Page = 'home' | 'chat' | 'projects' | 'delivery' | 'audit' | 'settings';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
const win = () => getCurrentWindow();

// ---- Traffic light buttons ----
function TrafficLights({ maximized, setMaximized }: { maximized: boolean; setMaximized: (v: boolean) => void }) {
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
function ForgeLogo({ size = 38 }: { size?: number }) {
  return (
    <img
      src={logoUrl}
      width={size}
      height={size}
      alt="AutoForge"
      style={{ borderRadius: size * 0.29, display: 'block' }}
      draggable={false}
    />
  );
}

const NAV: { id: Page; name: string; ic: string }[] = [
  { id: 'home',     name: '主页',     ic: 'home' },
  { id: 'chat',     name: '会议室',   ic: 'chat' },
  { id: 'projects', name: '项目管理', ic: 'box' },
  { id: 'delivery', name: '交付流水线', ic: 'package' },
  { id: 'audit',    name: '功能审计', ic: 'audit' },
];

export default function App() {
  const [page,  setPage]  = useState<Page>(() => {
    const saved = sessionStorage.getItem('AutoForge:page') as Page | null;
    return saved && (['home', 'chat', 'projects', 'delivery', 'audit', 'settings'] as string[]).includes(saved) ? saved : 'home';
  });
  const [theme, setTheme] = useState<ThemeSelection>(() => parseTheme(localStorage.getItem(THEME_STORAGE_KEY)));
  const [health, setHealth] = useState<SystemHealth | null>(null);
  const [badges, setBadges] = useState({ chat: 0, audit: 0 });
  // Window maximize state drives the .os-window shadow gutter (collapse it when maximized).
  const [maximized, setMaximized] = useState(false);
  // Cross-page jump target: Dashboard → Audit (a requirement to open in 功能审计).
  const [auditTarget, setAuditTarget] = useState<{ projectId: string; issueId: string } | null>(null);
  const goToAudit = useCallback((target: { projectId: string; issueId: string }) => {
    setAuditTarget(target);
    setPage('audit');
    sessionStorage.setItem('AutoForge:page', 'audit');
  }, []);

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

  // Apply persisted nav-rail expand behavior on startup (Settings updates it live).
  useEffect(() => {
    applyRailMode(parseRailMode(localStorage.getItem(RAIL_STORAGE_KEY)));
  }, []);

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

  // Track maximize state so the window shadow gutter collapses when maximized.
  useEffect(() => {
    if (!isTauri) return;
    win().isMaximized().then(setMaximized);
    let unlisten: (() => void) | undefined;
    win().onResized(() => win().isMaximized().then(setMaximized))
         .then(fn => { unlisten = fn; });
    return () => unlisten?.();
  }, []);

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
    <div className={'os-window' + (maximized ? ' maximized' : '')}>
      {/* Custom titlebar — onMouseDown triggers window drag */}
      <div className="os-titlebar" onMouseDown={handleTitlebarMouseDown}>
        <TrafficLights maximized={maximized} setMaximized={setMaximized} />
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
          <div className="rail-inner">
          <div className="rail-logo" title="AutoForge">
            <span className="rail-ic"><ForgeLogo size={32} /></span>
            <span className="rail-label rail-wordmark">AUTO<b>FORGE</b></span>
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
              <span className="rail-ic">
                <Icon name={n.ic} size={23} />
                {badge > 0 && page !== n.id && <span className="rail-badge">{badge}</span>}
              </span>
              <span className="rail-label">{n.name}</span>
            </button>
            );
          })}
          <div className="rail-spacer" />
          <button
            className="rail-item"
            title={theme.mode === 'dark' ? '切换浅色' : '切换深色'}
            onClick={() => setTheme(t => ({ ...t, mode: oppositeMode(t.mode) }))}
          >
            <span className="rail-ic"><Icon name={theme.mode === 'dark' ? 'sun' : 'moon'} size={23} /></span>
            <span className="rail-label">{theme.mode === 'dark' ? '浅色模式' : '深色模式'}</span>
          </button>
          <button
            className={'rail-item' + (page === 'settings' ? ' active' : '')}
            onClick={() => { setPage('settings'); sessionStorage.setItem('AutoForge:page', 'settings'); }}
            title="设置"
          >
            <span className="rail-ic"><Icon name="settings" size={23} /></span>
            <span className="rail-label">设置</span>
          </button>
          <div className="rail-item rail-me">
            <span className="rail-ic"><MeAvatar size={34} /></span>
            <span className="rail-label">我的账户</span>
          </div>
          </div>
        </div>

        {page === 'home'     && <Dashboard onOpenInAudit={goToAudit} />}
        {page === 'chat'     && <ConversationsPage />}
        {page === 'projects' && <ProjectsPage />}
        {page === 'delivery' && <DeliveryPage />}
        {page === 'audit'    && <AuditPage target={auditTarget} onTargetConsumed={() => setAuditTarget(null)} />}
        {page === 'settings' && <SettingsPage theme={theme} onThemeChange={setTheme} />}
      </div>
    </div>
  );
}
