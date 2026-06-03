/* ============================================================
   AutoForge — Settings (LLM configs + Agent configs)
   ============================================================ */

function Switch({ on, onToggle }) {
  return <button className={'switch' + (on ? ' on' : '')} onClick={onToggle}><i /></button>;
}

function LLMSettings() {
  const [configs, setConfigs] = useState(AF.LLM_CONFIGS);
  const [exp, setExp] = useState('l1');
  return (
    <div className="set-inner rise">
      <div className="set-h">LLM 配置</div>
      <div className="set-desc">管理多个大模型连接。每个 Agent 可指派不同的 LLM —— 分析用高推理模型，执行用快速模型，输入消毒可用本地小模型以降低延迟与成本。</div>
      {configs.map(c => (
        <div className="cfg-card" key={c.id} style={exp === c.id ? { borderColor: 'var(--ember-tint-strong)' } : {}}>
          <div className="cfg-top" onClick={() => setExp(exp === c.id ? null : c.id)} style={{ cursor: 'pointer' }}>
            <div className="cfg-logo" style={{ background: c.color }}><Icon name="brain" size={20} /></div>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div className="cfg-name">{c.name}</div>
              <div className="cfg-sub">{c.provider} · {c.model}</div>
            </div>
            <span className={'chip ' + (c.active ? 'green' : '')}>{c.active ? '● 已启用' : '未启用'}</span>
            <Icon name={exp === c.id ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)', marginLeft: 4 }} />
          </div>
          {exp === c.id && (
            <div className="cfg-fields rise">
              <div className="field"><label>Provider</label><select defaultValue={c.provider}><option>Anthropic</option><option>OpenAI</option><option>Ollama</option><option>Azure</option><option>自定义</option></select></div>
              <div className="field"><label>Model</label><input className="mono" defaultValue={c.model} /></div>
              <div className="field full"><label>API Endpoint</label><input className="mono" defaultValue={c.endpoint} /></div>
              <div className="field full"><label><Icon name="key" size={11} style={{ verticalAlign: -1, marginRight: 4 }} />API Key</label><input className="mono" defaultValue={c.key} type="text" /></div>
              <div className="field"><label>上下文窗口</label><input defaultValue={c.ctx} /></div>
              <div className="field"><label>Temperature</label><input defaultValue={c.temp} /></div>
              <div className="field full" style={{ flexDirection: 'row', alignItems: 'center', gap: 12, marginTop: 4 }}>
                <Switch on={c.active} onToggle={() => setConfigs(cs => cs.map(x => x.id === c.id ? { ...x, active: !x.active } : x))} />
                <span style={{ fontSize: 13, color: 'var(--text-2)' }}>启用此连接</span>
                <button className="btn btn-sm btn-danger" style={{ marginLeft: 'auto' }}><Icon name="trash" size={13} />删除</button>
                <button className="btn btn-sm"><Icon name="zap" size={13} />测试连接</button>
              </div>
            </div>
          )}
        </div>
      ))}
      <div className="cfg-card add"><Icon name="plus" size={18} />添加 LLM 配置</div>
    </div>
  );
}

function AgentSettings() {
  const [agents, setAgents] = useState(AF.AGENTS.map(a => ({ ...a })));
  const [exp, setExp] = useState('analyst');
  const [analystId, setAnalystId] = useState('analyst');
  const [testerId, setTesterId] = useState('tester');
  const llmNames = AF.LLM_CONFIGS.map(l => l.name);

  return (
    <div className="set-inner rise">
      <div className="set-h">Agent 配置</div>
      <div className="set-desc">为不同职能配置 Agent：选择使用的 LLM、编写系统提示词。并可指派某个 Agent 担任 AutoForge 流水线中的<b style={{ color: 'var(--text)' }}>需求分析 Agent</b> 或 <b style={{ color: 'var(--text)' }}>测试 Agent</b>。</div>

      {/* role assignment */}
      <div className="panel" style={{ marginBottom: 22 }}>
        <div className="panel-head"><div className="panel-title"><Icon name="sliders" size={16} style={{ color: 'var(--ember)' }} />流水线角色指派</div></div>
        <div style={{ padding: '4px 18px 14px' }}>
          <div className="assign-row">
            <div className="cfg-logo" style={{ background: 'var(--violet)', width: 34, height: 34 }}><Icon name="search" size={17} /></div>
            <div className="assign-info">
              <div className="assign-title">需求分析 Agent</div>
              <div className="assign-desc">在审核节点 1 前评估真实性、可行性、优先级</div>
            </div>
            <select className="field" style={{ width: 180, background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 9, padding: '8px 10px', color: 'var(--text)', fontSize: 13 }} value={analystId} onChange={e => setAnalystId(e.target.value)}>
              {agents.map(a => <option key={a.id} value={a.id}>{a.name}</option>)}
            </select>
          </div>
          <div className="assign-row">
            <div className="cfg-logo" style={{ background: 'var(--green)', width: 34, height: 34 }}><Icon name="flask" size={17} /></div>
            <div className="assign-info">
              <div className="assign-title">测试 Agent</div>
              <div className="assign-desc">合并后被动响应 + 每日主动巡检，缺陷自动入队</div>
            </div>
            <select className="field" style={{ width: 180, background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 9, padding: '8px 10px', color: 'var(--text)', fontSize: 13 }} value={testerId} onChange={e => setTesterId(e.target.value)}>
              {agents.map(a => <option key={a.id} value={a.id}>{a.name}</option>)}
            </select>
          </div>
        </div>
      </div>

      <div className="sec-kicker" style={{ marginBottom: 12 }}>全部 Agent · {agents.length}</div>
      {agents.map(a => {
        const roles = [];
        if (a.id === analystId) roles.push('需求分析');
        if (a.id === testerId) roles.push('测试');
        return (
          <div className="cfg-card" key={a.id} style={exp === a.id ? { borderColor: 'var(--ember-tint-strong)' } : {}}>
            <div className="cfg-top" onClick={() => setExp(exp === a.id ? null : a.id)} style={{ cursor: 'pointer' }}>
              <Avatar agent={a} size={40} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="cfg-name" style={{ display: 'flex', alignItems: 'center', gap: 8 }}>{a.name}
                  {roles.map(r => <span key={r} className="chip ember" style={{ padding: '1px 7px', fontSize: 10 }}>{r} Agent</span>)}
                </div>
                <div className="cfg-sub">{a.en} · {a.llm}</div>
              </div>
              <Icon name={exp === a.id ? 'chevDown' : 'chevRight'} size={18} style={{ color: 'var(--text-3)' }} />
            </div>
            {exp === a.id && (
              <div className="rise" style={{ marginTop: 15 }}>
                <div className="cfg-fields">
                  <div className="field"><label>Agent 名称</label><input defaultValue={a.name} /></div>
                  <div className="field"><label>使用的 LLM</label><select defaultValue={a.llm}>{llmNames.map(n => <option key={n}>{n}</option>)}</select></div>
                  <div className="field full"><label>职责标签</label><input defaultValue={a.role} /></div>
                  <div className="field full"><label>系统提示词 · System Prompt</label><textarea className="mono" rows={4} defaultValue={a.system} /></div>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 18, marginTop: 14, paddingTop: 14, borderTop: '1px solid var(--border)' }}>
                  <label style={{ display: 'flex', alignItems: 'center', gap: 9, fontSize: 13, color: 'var(--text-2)', cursor: 'pointer' }}>
                    <Switch on={a.id === analystId} onToggle={() => setAnalystId(a.id)} />指派为需求分析 Agent
                  </label>
                  <label style={{ display: 'flex', alignItems: 'center', gap: 9, fontSize: 13, color: 'var(--text-2)', cursor: 'pointer' }}>
                    <Switch on={a.id === testerId} onToggle={() => setTesterId(a.id)} />指派为测试 Agent
                  </label>
                  <button className="btn btn-sm btn-danger" style={{ marginLeft: 'auto' }}><Icon name="trash" size={13} />删除</button>
                </div>
              </div>
            )}
          </div>
        );
      })}
      <div className="cfg-card add"><Icon name="plus" size={18} />添加 Agent</div>
    </div>
  );
}

function SettingsPage() {
  const [sec, setSec] = useState('llm');
  const items = [
    { id: 'llm', name: 'LLM 配置', ic: 'brain' },
    { id: 'agents', name: 'Agent 配置', ic: 'bot' },
    { id: 'concurrency', name: '并发与流控', ic: 'cpu' },
    { id: 'security', name: '安全与权限', ic: 'shield' },
    { id: 'specs', name: '规范文档', ic: 'file' },
    { id: 'about', name: '关于 AutoForge', ic: 'zap' },
  ];
  return (
    <div className="content">
      <div className="audit-top" style={{ height: 56 }}>
        <div className="eyebrow" style={{ fontSize: 17 }}><span className="en">SETTINGS</span><span className="cn">· 设置</span></div>
      </div>
      <div className="set-wrap">
        <div className="set-nav">
          {items.map(it => (
            <div key={it.id} className={'set-nav-item' + (sec === it.id ? ' active' : '')} onClick={() => setSec(it.id)}>
              <Icon name={it.ic} size={18} />{it.name}
            </div>
          ))}
        </div>
        <div className="set-body scroll">
          {sec === 'llm' && <LLMSettings />}
          {sec === 'agents' && <AgentSettings />}
          {!['llm', 'agents'].includes(sec) && (
            <div className="empty" style={{ height: '100%' }}><Icon name={items.find(i => i.id === sec).ic} /><div>{items.find(i => i.id === sec).name}<br /><span style={{ fontSize: 12 }}>此面板为占位，可按需扩展</span></div></div>
          )}
        </div>
      </div>
    </div>
  );
}
window.SettingsPage = SettingsPage;
