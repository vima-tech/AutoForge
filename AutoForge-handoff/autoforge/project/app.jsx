/* ============================================================
   AutoForge — App shell (macOS window + nav rail + routing)
   ============================================================ */

function ForgeLogo({ size = 22 }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="#2a1607" strokeWidth="2.1" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 20h10" />
      <path d="M14 14l6-6-3-3-6 6z" />
      <path d="M8 20l4-9 3 3-3 6" />
      <path d="M17 5l2-2" />
    </svg>
  );
}

const NAV = [
  { id: 'home', name: '主页', ic: 'home' },
  { id: 'chat', name: '对话', ic: 'chat', badge: 3 },
  { id: 'audit', name: '功能审计', ic: 'audit', badge: 4 },
];

const ACCENTS = {
  ember:  { m: '#e8772e', s: '#f29a55', d: '#c2571a', g: 'linear-gradient(135deg,#f4a93c,#e8772e 55%,#d65f1c)' },
  blue:   { m: '#4f8ed1', s: '#79b0e6', d: '#3a72ad', g: 'linear-gradient(135deg,#74b6f0,#4f8ed1 55%,#3a72ad)' },
  green:  { m: '#4f9d6b', s: '#74c293', d: '#3a7d52', g: 'linear-gradient(135deg,#74c293,#4f9d6b 55%,#3a7d52)' },
  violet: { m: '#8b7ad8', s: '#ad9cf0', d: '#6f5cc0', g: 'linear-gradient(135deg,#ad9cf0,#8b7ad8 55%,#6f5cc0)' },
};
function hexA(hex, a) {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${a})`;
}

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "accent": "ember",
  "bubble": "rounded",
  "listWidth": 300,
  "motion": true
}/*EDITMODE-END*/;

function TweakDeck({ t, setTweak }) {
  return (
    <TweaksPanel>
      <TweakSection label="品牌 Brand" />
      <TweakColor label="主题色 Accent" value={ACCENTS[t.accent].m}
        options={[ACCENTS.ember.m, ACCENTS.blue.m, ACCENTS.green.m, ACCENTS.violet.m]}
        onChange={(v) => setTweak('accent', Object.keys(ACCENTS).find(k => ACCENTS[k].m === v) || 'ember')} />
      <TweakSection label="布局 Layout" />
      <TweakRadio label="气泡圆角 Bubble" value={t.bubble} options={['rounded', 'sharp']}
        onChange={(v) => setTweak('bubble', v)} />
      <TweakSlider label="列表栏宽 List" value={t.listWidth} min={260} max={360} step={10} unit="px"
        onChange={(v) => setTweak('listWidth', v)} />
      <TweakToggle label="入场动画 Motion" value={t.motion}
        onChange={(v) => setTweak('motion', v)} />
    </TweaksPanel>
  );
}

function App() {
  const [page, setPage] = useState('home');
  const [theme, setTheme] = useState('dark');
  const [t, setTweak] = useTweaks(TWEAK_DEFAULTS);

  useEffect(() => { document.documentElement.setAttribute('data-theme', theme); }, [theme]);

  useEffect(() => {
    const r = document.documentElement.style;
    const a = ACCENTS[t.accent] || ACCENTS.ember;
    r.setProperty('--ember', a.m);
    r.setProperty('--ember-soft', a.s);
    r.setProperty('--ember-deep', a.d);
    r.setProperty('--molten', a.g);
    r.setProperty('--ember-tint', hexA(a.m, .14));
    r.setProperty('--ember-tint-strong', hexA(a.m, .24));
    r.setProperty('--list-w', t.listWidth + 'px');
    document.documentElement.setAttribute('data-bubble', t.bubble);
    document.documentElement.setAttribute('data-motion', t.motion ? 'on' : 'off');
  }, [t.accent, t.listWidth, t.bubble, t.motion]);

  // expose for tweaks
  useEffect(() => {
    window.__afSetTheme = setTheme;
    window.__afSetPage = setPage;
  }, []);

  return (
    <div className="os-stage">
      <div className="os-window">
        {/* titlebar */}
        <div className="os-titlebar">
          <div className="traffic"><i className="r" /><i className="y" /><i className="g" /></div>
          <div className="tb-title">AUTO<b>FORGE</b> · 通用软件工厂</div>
          <div className="tb-right">
            <span className="chip green" style={{ padding: '3px 9px' }}><span className="dot green" style={{ width: 6, height: 6, boxShadow: 'none' }} />流水线运行中</span>
          </div>
        </div>

        <div className="os-body">
          {/* nav rail */}
          <div className="rail">
            <div className="rail-logo" title="AutoForge"><ForgeLogo size={22} /></div>
            {NAV.map(n => (
              <button key={n.id} className={'rail-item' + (page === n.id ? ' active' : '')} onClick={() => setPage(n.id)} title={n.name}>
                <Icon name={n.ic} size={23} />
                {n.badge && page !== n.id && <span className="rail-badge">{n.badge}</span>}
              </button>
            ))}
            <div className="rail-spacer" />
            <button className="rail-item" title={theme === 'dark' ? '切换浅色' : '切换深色'} onClick={() => setTheme(t => t === 'dark' ? 'light' : 'dark')}>
              <Icon name={theme === 'dark' ? 'sun' : 'moon'} size={22} />
            </button>
            <button className={'rail-item' + (page === 'settings' ? ' active' : '')} onClick={() => setPage('settings')} title="设置">
              <Icon name="settings" size={23} />
            </button>
            <div style={{ marginTop: 6 }}><MeAvatar size={38} /></div>
          </div>

          {/* page */}
          {page === 'home' && <Dashboard />}
          {page === 'chat' && <ConversationsPage />}
          {page === 'audit' && <AuditPage />}
          {page === 'settings' && <SettingsPage />}
        </div>
      </div>
      <TweakDeck t={t} setTweak={setTweak} />
    </div>
  );
}

ReactDOM.createRoot(document.getElementById('root')).render(<App />);
