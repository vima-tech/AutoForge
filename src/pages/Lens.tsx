import React, { useEffect, useState, useCallback } from 'react';
import Icon from '../components/Icon';
import Select from '../components/Select';
import {
  listActiveProjects, listLensPresets, assembleLens, fetchContextContent,
  listConversations, citeContextToConversation,
  type Project, type LensPreset, type ContextItem, type Conversation,
} from '../services';

/**
 * L4 角色取景框（Lens）——方法论平台/基质 §4.3。
 * 同一上下文基质配不同取景框即得不同「角色台子」：选项目 + 角色 → 装配该角色默认纳入的
 * 上下文条目（需求/编码日志/审核意见/草稿/发言…），点开懒取正文。角色收敛在**默认值**上，
 * 不硬隔离——底层基质随时可展开取用更多。
 */
export default function Lens() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [projectId, setProjectId] = useState('');
  const [presets, setPresets] = useState<LensPreset[]>([]);
  const [role, setRole] = useState('coding');
  const [items, setItems] = useState<ContextItem[]>([]);
  const [busy, setBusy] = useState(false);
  const [openId, setOpenId] = useState('');
  const [content, setContent] = useState<Record<string, string>>({});
  // 引用到会议室：目标群聊 + 反馈。
  const [convs, setConvs] = useState<Conversation[]>([]);
  const [citeTo, setCiteTo] = useState('');
  const [cited, setCited] = useState('');

  useEffect(() => {
    listActiveProjects().then(ps => {
      setProjects(ps);
      if (ps[0]) setProjectId(p => p || ps[0].id);
    }).catch(() => {});
    listLensPresets().then(setPresets).catch(() => {});
    listConversations().then(setConvs).catch(() => {});
  }, []);

  // 可引用目标：绑定当前项目的群聊（未绑定项目的群聊也可选，兜底全部群聊）。
  const targetConvs = convs.filter(c => c.conv_type === 'group' && (!c.project_id || c.project_id === projectId));
  const cite = async (it: ContextItem) => {
    if (!citeTo) { setCited('请先在上方选择目标会议室'); return; }
    try { await citeContextToConversation(citeTo, it.id); setCited(`已引用到会议室 · ${it.title || it.id}`); }
    catch (e) { setCited(String(e)); }
  };

  const assemble = useCallback(() => {
    if (!projectId) { setItems([]); return; }
    setBusy(true);
    assembleLens(projectId, role)
      .then(setItems)
      .catch(() => setItems([]))
      .finally(() => setBusy(false));
  }, [projectId, role]);

  useEffect(() => { assemble(); }, [assemble]);

  const toggle = async (it: ContextItem) => {
    if (openId === it.id) { setOpenId(''); return; }
    setOpenId(it.id);
    if (content[it.id] === undefined) {
      try {
        const c = await fetchContextContent(it.id, 4000);
        setContent(m => ({ ...m, [it.id]: c }));
      } catch { setContent(m => ({ ...m, [it.id]: '（读取失败）' })); }
    }
  };

  const activePreset = presets.find(p => p.role === role);

  return (
    <div className="audit-top" style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div className="audit-head" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, padding: '14px 18px', flexShrink: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <Icon name="layers" size={18} style={{ color: 'var(--ember)' }} />
          <span style={{ fontFamily: 'var(--font-display)', fontSize: 'var(--text-heading)', fontWeight: 700 }}>取景框</span>
          <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '.12em' }}>
            角色 · 上下文基质
          </span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <div style={{ minWidth: 180 }}>
            <Select className="sm" value={citeTo} onChange={setCiteTo}
              options={targetConvs.map(c => ({ value: c.id, label: c.name || '未命名群聊' }))}
              placeholder="引用到会议室…" />
          </div>
          <div style={{ minWidth: 180 }}>
            <Select className="sm" value={projectId} onChange={setProjectId}
              options={projects.map(p => ({ value: p.id, label: p.name }))} placeholder="选择项目" />
          </div>
        </div>
      </div>
      {cited && (
        <div className="chip green" style={{ margin: '0 18px 10px', display: 'inline-block', padding: '6px 12px' }}>{cited}</div>
      )}

      {/* 角色台子切换（seg：呼应 rail 激活态） */}
      <div className="seg" style={{ margin: '0 18px 12px', flexShrink: 0, alignSelf: 'flex-start' }}>
        {presets.map(p => (
          <button key={p.id} className={'seg-btn' + (role === p.role ? ' on' : '')}
            onClick={() => setRole(p.role)}>{p.name}</button>
        ))}
      </div>

      <div style={{ flex: 1, overflowY: 'auto', padding: '0 18px 18px', display: 'flex', flexDirection: 'column', gap: 8 }}>
        {activePreset && (
          <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-faint)', letterSpacing: '.03em' }}>
            默认纳入：{activePreset.include.length ? activePreset.include.join(' · ') : '全部来源'}
          </div>
        )}
        {busy && <div className="empty-compact" style={{ padding: 18 }}>装配中…</div>}
        {!busy && items.length === 0 && (
          <div className="empty-compact" style={{ padding: 18 }}>该取景框下暂无上下文条目（基质随写入路径逐步填充）</div>
        )}
        {items.map(it => (
          <div key={it.id} className="panel" style={{ padding: 0, overflow: 'hidden' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '10px 14px' }}>
              <button onClick={() => toggle(it)}
                style={{ flex: 1, display: 'flex', alignItems: 'center', gap: 10, background: 'transparent', border: 'none', cursor: 'pointer', textAlign: 'left', minWidth: 0 }}>
                <span className="chip">{it.source_kind}</span>
                <span style={{ flex: 1, fontSize: 'var(--text-label)', color: 'var(--text)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{it.title || it.id}</span>
                {it.trust === 'external_untrusted' && <span className="chip amber">外部</span>}
              </button>
              <button className="btn btn-sm" style={{ flexShrink: 0 }} onClick={() => cite(it)}
                disabled={!citeTo} title={citeTo ? '引用到所选会议室' : '先选目标会议室'}>
                <Icon name="chat" size={13} />引用
              </button>
              <span style={{ cursor: 'pointer', display: 'inline-flex', color: 'var(--text-3)' }} onClick={() => toggle(it)}>
                <Icon name={openId === it.id ? 'chevron-down' : 'chevron-right'} size={14} />
              </span>
            </div>
            {openId === it.id && (
              <div style={{ padding: '10px 14px', borderTop: '1px solid var(--border)', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-2)', whiteSpace: 'pre-wrap', wordBreak: 'break-word', lineHeight: 'var(--leading-relaxed)', maxHeight: 320, overflowY: 'auto' }}>
                {content[it.id] ?? '读取中…'}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
