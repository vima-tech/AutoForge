/* ============================================================
   AutoForge — Function Audit (review portal: 审核节点 2)
   ============================================================ */

function AuditList({ project, setProject, active, onSelect }) {
  const projs = AF.AUDIT_PROJECTS;
  const [open, setOpen] = useState(false);
  const sevColor = { critical: 'red', high: 'amber', medium: 'blue', low: 'green' };
  const catColor = { Bug: 'red', Feature: 'ember', Improvement: 'blue', Debt: 'violet' };
  return (
    <div className="list-col">
      <div className="audit-proj">
        <div className="proj-select" onClick={() => setOpen(o => !o)}>
          <div className="proj-logo" style={{ background: project.color }}>{project.name[0]}</div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div className="proj-name">{project.name}</div>
            <div className="proj-meta">{project.lang}</div>
          </div>
          <Icon name="chevDown" size={16} style={{ color: 'var(--text-3)' }} />
        </div>
        {open && (
          <div style={{ background: 'var(--bg-3)', border: '1px solid var(--border-strong)', borderRadius: 12, padding: 6, marginTop: -4 }}>
            {projs.map(p => (
              <div key={p.name} className="mention-row" onClick={() => { setProject(p); setOpen(false); }}>
                <div className="proj-logo" style={{ background: p.color, width: 30, height: 30, fontSize: 13 }}>{p.name[0]}</div>
                <div className="nm">{p.name}</div>
                {p.name === project.name && <Icon name="check" size={15} style={{ marginLeft: 'auto', color: 'var(--ember)' }} />}
              </div>
            ))}
          </div>
        )}
        {project.live
          ? <div className="preview-url"><span className="live" /><span style={{ flex: 1 }}>{project.preview}</span><Icon name="external" size={14} style={{ color: 'var(--text-3)' }} /></div>
          : <button className="btn btn-primary" style={{ justifyContent: 'center' }}><Icon name="play" size={15} />启动项目预览</button>}
      </div>
      <div className="list-group-label">待审核需求 · 审核节点 2</div>
      <div className="list-body scroll" style={{ paddingTop: 0 }}>
        {AF.REQUIREMENTS.map(r => (
          <div key={r.id} className={'req-item' + (active === r.id ? ' active' : '')} onClick={() => onSelect(r.id)}>
            <div className="req-item-top">
              <span className="req-id">{r.id}</span>
              <span className={'chip ' + catColor[r.cat]} style={{ padding: '1px 7px', fontSize: 10 }}>{r.cat}</span>
              <span className={'dot ' + (sevColor[r.sev] === 'red' ? 'amber' : 'green')} style={{ marginLeft: 'auto', background: `var(--${r.sev === 'critical' ? 'red' : r.sev === 'high' ? 'amber' : 'blue'})` }} />
            </div>
            <div className="req-title">{r.title}</div>
            <div className="req-foot">
              <span className={'iter-badge' + (r.iter >= 3 ? ' hot' : '')}>迭代 {r.iter}</span>
              <span>{r.files} 文件</span>
              <span style={{ marginLeft: 'auto' }}>{r.time}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function UnifiedDiff() {
  return (
    <div className="diff">
      {AF.DIFF_HUNKS.map((h, hi) => (
        <div key={hi}>
          <div className="diff-hunk">{h.hunk}</div>
          {h.lines.map((l, i) => (
            <div key={i} className={'diff-line ' + (l.t === 'add' ? 'add' : l.t === 'del' ? 'del' : '')}>
              <span className="gut">{l.n1}</span>
              <span className="gut">{l.n2}</span>
              <span className="code">{l.t === 'add' ? '+ ' : l.t === 'del' ? '- ' : '  '}{l.code}</span>
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

function SplitDiff() {
  return (
    <div className="diff">
      {AF.DIFF_HUNKS.map((h, hi) => (
        <div key={hi}>
          <div className="diff-hunk">{h.hunk}</div>
          <div className="diff-split-wrap">
            <div>
              {h.lines.filter(l => l.t !== 'add').map((l, i) => (
                <div key={i} className={'diff-line ' + (l.t === 'del' ? 'del' : '')}>
                  <span className="gut">{l.n1}</span>
                  <span className="code">{l.code}</span>
                </div>
              ))}
            </div>
            <div>
              {h.lines.filter(l => l.t !== 'del').map((l, i) => (
                <div key={i} className={'diff-line ' + (l.t === 'add' ? 'add' : '')}>
                  <span className="gut">{l.n2}</span>
                  <span className="code">{l.code}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

/* fake mini app screens for the preview frames */
function PreviewScreen({ variant }) {
  const tags = variant === 'cr'
    ? ['全部', '待办', '已完成']
    : null;
  return (
    <div className="prev-screen">
      <div style={{ fontFamily: 'var(--font-display)', fontWeight: 800, fontSize: 16, marginBottom: 10 }}>智能助手</div>
      {tags && (
        <div style={{ display: 'flex', gap: 7, marginBottom: 12 }}>
          {tags.map((t, i) => (
            <span key={t} style={{ fontSize: 11, fontWeight: 600, padding: '4px 11px', borderRadius: 99, background: i === 0 ? 'var(--molten)' : 'var(--bg-2)', color: i === 0 ? '#2a1607' : 'var(--text-2)', border: i === 0 ? 'none' : '1px solid var(--border)' }}>{t}</span>
          ))}
        </div>
      )}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        {[1, 2, 3].map(i => (
          <div key={i} style={{ display: 'flex', gap: 9, padding: 9, background: 'var(--bg-2)', borderRadius: 10, border: '1px solid var(--border)' }}>
            <div style={{ width: 26, height: 26, borderRadius: 7, background: ['#4f8ed1', '#4f9d6b', '#8b7ad8'][i - 1], flex: 'none' }} />
            <div style={{ flex: 1 }}>
              <div style={{ height: 7, width: '55%', background: 'var(--text-faint)', borderRadius: 4, opacity: .5 }} />
              <div style={{ height: 6, width: '82%', background: 'var(--border-strong)', borderRadius: 4, marginTop: 6 }} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function AuditPage() {
  const [project, setProject] = useState(AF.AUDIT_PROJECTS[0]);
  const [active, setActive] = useState('CR-042');
  const [diffMode, setDiffMode] = useState('unified');
  const [tab, setTab] = useState('report'); // report | diff
  const [advice, setAdvice] = useState('');
  const [decided, setDecided] = useState(null);
  const req = AF.REQUIREMENTS.find(r => r.id === active);
  const r = AF.REPORT;

  return (
    <>
      <AuditList project={project} setProject={setProject} active={active} onSelect={(id) => { setActive(id); setDecided(null); }} />
      <div className="content">
        <div className="audit-top">
          <div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <span className="req-id" style={{ fontSize: 13 }}>{req.id}</span>
              <span style={{ fontWeight: 700, fontSize: 15 }}>{req.title}</span>
            </div>
            <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>由 {req.author} 创建 · {req.time} · 迭代 {req.iter} 轮</div>
          </div>
          <div className="audit-decide">
            {decided
              ? <span className={'chip ' + (decided === 'approve' ? 'green' : decided === 'reject' ? 'red' : 'amber')} style={{ padding: '7px 14px', fontSize: 13 }}>
                  <Icon name={decided === 'approve' ? 'check' : decided === 'reject' ? 'x' : 'refresh'} size={14} />
                  {decided === 'approve' ? '已批准 · 合并到 dev' : decided === 'reject' ? '已拒绝 · 销毁 worktree' : '已退回 · 追加建议重做'}
                </span>
              : <>
                  <button className="btn btn-danger" onClick={() => setDecided('reject')}><Icon name="x" size={15} />拒绝</button>
                  <button className="btn" onClick={() => setDecided('revise')}><Icon name="refresh" size={15} />修改</button>
                  <button className="btn btn-primary" onClick={() => setDecided('approve')}><Icon name="check" size={15} />批准合并</button>
                </>}
          </div>
        </div>

        <div className="audit-split">
          {/* LEFT: report + diff */}
          <div className="audit-left scroll">
            <div style={{ padding: '14px 22px 0', display: 'flex', alignItems: 'center', gap: 10 }}>
              <div className="seg">
                <button className={tab === 'report' ? 'on' : ''} onClick={() => setTab('report')}>实现报告</button>
                <button className={tab === 'diff' ? 'on' : ''} onClick={() => setTab('diff')}>代码 Diff</button>
              </div>
              {tab === 'diff' && (
                <div className="seg" style={{ marginLeft: 'auto' }}>
                  <button className={diffMode === 'unified' ? 'on' : ''} onClick={() => setDiffMode('unified')}><Icon name="rows" size={13} style={{ verticalAlign: -2, marginRight: 4 }} />统一</button>
                  <button className={diffMode === 'split' ? 'on' : ''} onClick={() => setDiffMode('split')}><Icon name="columns" size={13} style={{ verticalAlign: -2, marginRight: 4 }} />分栏</button>
                </div>
              )}
            </div>

            {tab === 'report' ? (
              <div className="report">
                {req.iter >= 3 && (
                  <div className="iter-warn">
                    <Icon name="alert" size={20} />
                    <div>已迭代 <b>{req.iter}</b> 轮（软上限 3）。建议考虑：手动介入直接修改代码、重新描述需求并提供更明确的约束，或拆分为更小的子需求。</div>
                  </div>
                )}
                <h2><Icon name="zap" size={18} style={{ color: 'var(--ember)' }} />改动摘要</h2>
                <p>{r.summary}</p>
                <h2><Icon name="file" size={18} style={{ color: 'var(--blue)' }} />修改文件</h2>
                <div>
                  {r.files.map((f, i) => (
                    <span className="file-pill" key={i}>
                      <Icon name="file" size={13} />{f.name}
                      <span className="add">+{f.add}</span>{f.del > 0 && <span className="del">-{f.del}</span>}
                    </span>
                  ))}
                </div>
                <h2><Icon name="flask" size={18} style={{ color: 'var(--green)' }} />测试情况</h2>
                <p>新增 <b style={{ color: 'var(--text)' }}>{r.tests.added}</b> 个测试 · 全部 <span style={{ color: 'var(--green-soft)' }}>通过 ✓</span> · 覆盖率 <b style={{ color: 'var(--text)' }}>{r.tests.cov}</b>（阈值 80%）</p>
                <h2><Icon name="shield" size={18} style={{ color: 'var(--violet)' }} />潜在风险</h2>
                <p>{r.risk}</p>
              </div>
            ) : (
              <div>
                <div className="diff-toolbar">
                  <Icon name="file" size={15} style={{ color: 'var(--text-3)' }} />
                  <span className="diff-file">app/web/Assistant.jsx</span>
                  <span className="chip green" style={{ marginLeft: 'auto', padding: '2px 8px' }}>+52</span>
                  <span className="chip red" style={{ padding: '2px 8px' }}>-6</span>
                </div>
                {diffMode === 'unified' ? <UnifiedDiff /> : <SplitDiff />}
              </div>
            )}
          </div>

          {/* RIGHT: live preview compare + advice */}
          <div className="audit-right">
            <div className="prev-head">
              <Icon name="eye" size={16} style={{ color: 'var(--ember)' }} />
              <span style={{ fontWeight: 700, fontSize: 13.5 }}>实时预览对比</span>
              <button className="btn btn-sm btn-ghost" style={{ marginLeft: 'auto' }}><Icon name="external" size={13} />全屏</button>
            </div>
            <div className="scroll" style={{ flex: 1, overflowY: 'auto' }}>
              <div className="prev-frame">
                <div className="prev-bar">
                  <span className="prev-tag prod">生产 main</span>
                  <span className="url">{project.preview.replace('/dev', '/main')}</span>
                  <Icon name="external" size={13} style={{ color: 'var(--text-3)' }} />
                </div>
                <PreviewScreen variant="prod" />
              </div>
              <div className="prev-frame" style={{ borderColor: 'var(--ember-tint-strong)' }}>
                <div className="prev-bar">
                  <span className="prev-tag cr">本次改动 {req.id}</span>
                  <span className="url">{project.preview.replace('/dev', '/' + req.id.toLowerCase())}</span>
                  <button className="icon-btn" style={{ width: 24, height: 24, marginLeft: 'auto' }} title="重启容器全量重载"><Icon name="refresh" size={13} /></button>
                </div>
                <PreviewScreen variant="cr" />
              </div>
            </div>
            <div style={{ padding: '6px 12px 2px', fontFamily: 'var(--font-mono)', fontSize: 10.5, letterSpacing: '.1em', textTransform: 'uppercase', color: 'var(--text-faint)' }}>管理员建议 → Claude Code</div>
            <div className="advice-box">
              <textarea value={advice} onChange={e => setAdvice(e.target.value)} placeholder="在此输入给 Claude Code 的修改意见，点「修改」后进入新一轮迭代…" />
            </div>
          </div>
        </div>
      </div>
    </>
  );
}
window.AuditPage = AuditPage;
