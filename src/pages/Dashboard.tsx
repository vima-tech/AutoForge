import React from 'react';
import Icon from '../components/Icon';
import { DASH } from '../data/mock';

const SEV_COLOR: Record<string, string> = {
  critical: 'red', high: 'amber', medium: 'blue', low: 'green',
  Bug: 'red', Feature: 'ember', Improvement: 'blue', Debt: 'violet',
};

export default function Dashboard() {
  const d = DASH;
  return (
    <div className="dash scroll">
      <div className="dash-inner rise">
        {/* hero */}
        <div className="dash-hero">
          <div>
            <div className="sec-kicker" style={{ marginBottom: 8 }}>工厂总览 · FACTORY OVERVIEW</div>
            <div className="dash-hello">下午好，管理员</div>
            <div className="dash-sub">流水线运行正常 · 当前处于背压阶段 1 · 5 个 Agent 在线</div>
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button className="btn"><Icon name="bell" size={16} />通知</button>
            <button className="btn btn-primary"><Icon name="plus" size={16} />提交需求</button>
          </div>
        </div>

        {/* stats */}
        <div className="stat-grid">
          {d.stats.map((s, i) => (
            <div className="stat" key={i} style={{ animationDelay: `${i * 40}ms` }}>
              <div className="stat-ic" style={{ background: `color-mix(in oklab, ${s.color} 16%, transparent)`, color: s.color }}>
                <Icon name={s.ic} size={18} />
              </div>
              <div className="stat-val">{s.val}<span className="u">{s.unit}</span></div>
              <div className="stat-label">{s.label}</div>
              <div className={'stat-delta ' + (s.up ? 'up' : 'down')}>{s.delta}</div>
            </div>
          ))}
        </div>

        {/* pipeline */}
        <div className="panel" style={{ marginBottom: 16 }}>
          <div className="panel-head">
            <div className="eyebrow" style={{ fontSize: 14 }}>
              <span className="en">PIPELINE</span>
              <span className="cn">· 完整流水线</span>
            </div>
            <div className="sec-kicker">实时 · LIVE</div>
          </div>
          <div className="pipe scroll">
            {d.pipeline.map((p, i) => (
              <div className="pipe-stage" key={i}>
                <div className={'pipe-node ' + p.state}><Icon name={p.ic} size={20} /></div>
                <div className="pipe-cnt" style={{
                  color: p.state === 'active' ? 'var(--ember)' : p.state === 'warn' ? 'var(--amber)' : p.state === 'done' ? 'var(--green-soft)' : 'var(--text-2)',
                }}>{p.cnt}</div>
                <div className="pipe-name">{p.name}</div>
              </div>
            ))}
          </div>
        </div>

        {/* two cols */}
        <div className="dash-cols">
          {/* queue */}
          <div className="panel">
            <div className="panel-head">
              <div className="panel-title">
                <Icon name="inbox" size={17} style={{ color: 'var(--ember)' }} />
                需求队列 · 优先级排序
              </div>
              <button className="btn btn-sm btn-ghost">全部 17 条<Icon name="chevRight" size={14} /></button>
            </div>
            {d.queue.map((q, i) => (
              <div className="q-row" key={i}>
                <div className="q-pr">{q.pr}</div>
                <div className="q-main">
                  <div className="q-title">{q.title}</div>
                  <div className="q-meta">
                    <span className="req-id" style={{ color: 'var(--text-3)' }}>{q.id}</span>
                    <span className={'chip ' + (SEV_COLOR[q.cat] || 'blue')} style={{ padding: '1px 7px', fontSize: 10 }}>{q.cat}</span>
                    <span>· {q.proj}</span>
                  </div>
                </div>
                <span className={'chip ' + (q.stage.includes('审核') ? 'amber' : q.stage === '实现中' ? 'ember' : '')}>{q.stage}</span>
              </div>
            ))}
          </div>

          {/* concurrency + backpressure */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
            <div className="panel">
              <div className="panel-head">
                <div className="panel-title"><Icon name="cpu" size={17} style={{ color: 'var(--blue)' }} />并发槽位</div>
                <span className="sec-kicker">3 / 5</span>
              </div>
              <div style={{ padding: '14px 18px 18px' }}>
                <div className="slots">
                  <div className="slot busy">CR-042</div>
                  <div className="slot busy">CR-031</div>
                  <div className="slot busy warn">CR-049</div>
                  <div className="slot">空闲</div>
                  <div className="slot">空闲</div>
                </div>
                <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 12, lineHeight: 1.6 }}>
                  并发上限 <b style={{ color: 'var(--text)' }}>5</b> · 队列策略 <span style={{ fontFamily: 'var(--font-mono)' }}>priority</span>。槽位在管理员完成审核节点 2 后释放。
                </div>
              </div>
            </div>
            <div className="panel">
              <div className="panel-head">
                <div className="panel-title"><Icon name="play" size={15} style={{ color: 'var(--green)' }} />背压状态</div>
                <span className="chip green">阶段 1 · 正常</span>
              </div>
              <div style={{ padding: '14px 18px 18px' }}>
                <div className="bp-bar">
                  <div className="bp-seg" style={{ width: '25%', background: 'var(--green)' }} />
                  <div className="bp-seg" style={{ width: '60%', background: 'var(--bg-3)' }} />
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between', fontFamily: 'var(--font-mono)', fontSize: 10.5, color: 'var(--text-faint)' }}>
                  <span>积压 5 → 降速</span><span>积压 20 → 暂停</span>
                </div>
                <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 12, lineHeight: 1.6 }}>
                  待审核积压 <b style={{ color: 'var(--text)' }}>5</b> 条。系统全速运行，资源始终保留。
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* projects */}
        <div className="panel">
          <div className="panel-head">
            <div className="eyebrow" style={{ fontSize: 14 }}>
              <span className="en">PROJECTS</span>
              <span className="cn">· 在产项目</span>
            </div>
            <button className="btn btn-sm btn-ghost"><Icon name="plus" size={14} />接入项目</button>
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 1, background: 'var(--border)' }}>
            {d.projects.map((p, i) => (
              <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 13, padding: '15px 18px', background: 'var(--bg-2)' }}>
                <div className="proj-logo" style={{ background: p.color, width: 42, height: 42 }}>{p.name[0]}</div>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <span style={{ fontWeight: 700, fontSize: 14 }}>{p.name}</span>
                    <span className={'dot ' + (p.status === 'active' ? 'green' : 'gray')} />
                  </div>
                  <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>
                    {p.desc} · <span style={{ fontFamily: 'var(--font-mono)' }}>{p.lang}</span>
                  </div>
                </div>
                <div style={{ textAlign: 'right' }}>
                  <div style={{ fontFamily: 'var(--font-display)', fontWeight: 800, fontSize: 20 }}>{p.backlog}</div>
                  <div style={{ fontSize: 10.5, color: 'var(--text-faint)', fontFamily: 'var(--font-mono)' }}>BACKLOG</div>
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
