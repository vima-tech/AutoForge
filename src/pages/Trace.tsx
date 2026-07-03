import React, { useState, useEffect, useCallback, useMemo } from 'react';
import Icon from '../components/Icon';
import Select from '../components/Select';
import {
  listLlmTraces, getLlmTrace, listTraceAgentNames, clearLlmTraces,
  listAgentOutputs, getAgentOutput, listAgentOutputRoles, clearAgentOutputs,
  agentOutputFieldHealth, llmUsageStats,
  type LlmTraceSummary, type LlmTrace, type TraceFilter,
  type AgentOutputSummary, type AgentOutput, type FieldHealth,
  type LlmUsageStats,
} from '../services';

// span 类型 → chip 语义色（仅语义状态色，遵循设计系统）。
const KIND_CHIP: Record<string, string> = {
  agent: 'ember', llm: 'blue', tool: 'violet', mcp: 'green',
};
const KIND_LABEL: Record<string, string> = {
  agent: 'AGENT', llm: 'LLM', tool: 'TOOL', mcp: 'MCP',
};

// Innate 记忆召回命中标志：本次调用的 system_prompt 注入了「历史经验与技能」小节时显示。
// 用 violet（次级分类语义色）+ brain 图标，区别于 kind 语义色，表达「带着记忆在思考」。
function InnateChip({ size = 10 }: { size?: number }) {
  return (
    <span className="chip violet" style={{ fontSize: 'var(--text-micro)' }} title="本次调用注入了 Innate 记忆召回">
      <Icon name="brain" size={size} />INNATE
    </span>
  );
}

function fmtMs(ms: number | null): string {
  if (ms == null) return '—';
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}
function fmtTime(s: string): string {
  // 后端存 UTC（datetime('now')），补 Z 让本地时区正确显示。
  const d = new Date(s.includes('T') ? s : s.replace(' ', 'T') + 'Z');
  return isNaN(d.getTime()) ? s : d.toLocaleString();
}
function preview(s: string | null, n = 120): string {
  if (!s) return '';
  const t = s.replace(/\s+/g, ' ').trim();
  return t.length > n ? t.slice(0, n) + '…' : t;
}

// 把整条 trace 的全部 span 序列化为适合粘贴给 Claude 等 Agent 分析的 Markdown。
// 包含关联维度 + 每个 span 的 provider/model/tokens/latency/状态 + 完整出入参。
function buildTraceText(spans: LlmTrace[]): string {
  if (spans.length === 0) return '';
  const root = spans.find(s => s.kind === 'agent') ?? spans[0];
  const L: string[] = [];
  L.push('# AutoForge LLM Trace');
  L.push('');
  L.push(`- trace_id: ${root.trace_id}`);
  L.push(`- agent_role: ${root.agent_role ?? '-'}`);
  L.push(`- agent_name: ${root.agent_name ?? '-'}`);
  L.push(`- issue_id: ${root.issue_id ?? '-'}`);
  L.push(`- conversation_id: ${root.conversation_id ?? '-'}`);
  L.push(`- project_id: ${root.project_id ?? '-'}`);
  L.push(`- task_id: ${root.task_id ?? '-'}`);
  L.push(`- status: ${root.status}`);
  L.push(`- innate_recall: ${spans.some(s => s.innate_triggered) ? 'on' : 'off'}`);
  L.push(`- total_latency_ms: ${root.latency_ms ?? '-'}`);
  L.push(`- spans: ${spans.length}`);
  L.push(`- created_at: ${root.created_at}`);
  L.push('');
  L.push('请基于以下完整调用链路，定位问题/可优化点（提示词、工具使用、token 消耗、错误等），给出改进建议。');
  L.push('');
  spans.forEach((s, i) => {
    let meta: Record<string, unknown> | null = null;
    try { meta = s.metadata_json ? JSON.parse(s.metadata_json) : null; } catch { /* ignore */ }
    L.push('---');
    L.push(`## [${i + 1}] ${(KIND_LABEL[s.kind] ?? s.kind)} — ${s.name ?? s.model ?? s.agent_name ?? s.kind}`);
    const facts: string[] = [`status=${s.status}`];
    if (s.provider) facts.push(`provider=${s.provider}`);
    if (s.model) facts.push(`model=${s.model}`);
    if (s.prompt_tokens != null) facts.push(`prompt_tokens=${s.prompt_tokens}`);
    if (s.completion_tokens != null) facts.push(`completion_tokens=${s.completion_tokens}`);
    if (s.total_tokens != null) facts.push(`total_tokens=${s.total_tokens}`);
    if (s.latency_ms != null) facts.push(`latency_ms=${s.latency_ms}`);
    if (meta?.iteration != null) facts.push(`iteration=${meta.iteration}`);
    if (s.innate_triggered) facts.push('innate_recall=on');
    L.push(facts.join(' · '));
    if (s.error) { L.push(''); L.push('### ERROR'); L.push('```'); L.push(s.error); L.push('```'); }
    if (s.system_prompt) { L.push(''); L.push('### SYSTEM'); L.push('```'); L.push(s.system_prompt); L.push('```'); }
    if (s.input) { L.push(''); L.push('### INPUT'); L.push('```'); L.push(s.input); L.push('```'); }
    if (s.output) { L.push(''); L.push('### OUTPUT'); L.push('```'); L.push(s.output); L.push('```'); }
    L.push('');
  });
  return L.join('\n');
}

// 一个 span 的可展开卡片（入参/出参/系统提示/错误）。
function SpanRow({ span }: { span: LlmTrace }) {
  const [open, setOpen] = useState(span.kind === 'agent');
  const chip = KIND_CHIP[span.kind] ?? '';
  const meta = useMemo(() => {
    try { return span.metadata_json ? JSON.parse(span.metadata_json) : null; } catch { return null; }
  }, [span.metadata_json]);
  return (
    <div className="panel" style={{ marginBottom: 8 }}>
      <div className="panel-head" style={{ cursor: 'pointer', gap: 8 }} onClick={() => setOpen(o => !o)}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0, flex: 1 }}>
          <span className={'chip ' + chip} style={{ fontSize: 'var(--text-micro)' }}>{KIND_LABEL[span.kind] ?? span.kind}</span>
          <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', color: 'var(--text-2)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {span.name ?? span.model ?? span.agent_name ?? span.kind}
          </span>
          {span.status === 'error' && <span className="chip red" style={{ fontSize: 'var(--text-micro)' }}><Icon name="alert" size={10} />error</span>}
          {span.innate_triggered && <InnateChip />}
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexShrink: 0, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>
          {span.total_tokens != null && <span title="tokens">{span.total_tokens} tokens</span>}
          <span title="耗时">{fmtMs(span.latency_ms)}</span>
          <Icon name={open ? 'chevDown' : 'chevRight'} size={14} />
        </div>
      </div>
      {open && (
        <div style={{ padding: '10px 14px', display: 'flex', flexDirection: 'column', gap: 10 }}>
          <div style={{ display: 'flex', gap: 16, flexWrap: 'wrap', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>
            {span.provider && <span>provider: <b style={{ color: 'var(--text-2)' }}>{span.provider}</b></span>}
            {span.model && <span>model: <b style={{ color: 'var(--text-2)' }}>{span.model}</b></span>}
            {span.prompt_tokens != null && <span>in: {span.prompt_tokens}</span>}
            {span.completion_tokens != null && <span>out: {span.completion_tokens}</span>}
            {meta?.iteration != null && <span>iter: {meta.iteration}</span>}
            {meta?.stage && <span>stage: <b style={{ color: 'var(--text-2)' }}>{meta.stage}</b></span>}
            {meta?.terminated_by && (
              <span>收尾: <b style={{ color: meta.terminated_by === 'model_final' ? 'var(--text-2)' : 'var(--amber)' }}>{meta.terminated_by}</b></span>
            )}
            {meta?.iters != null && <span>轮数: {meta.iters}</span>}
            {meta?.tool_calls != null && (
              <span>工具: {meta.tool_calls}{meta.tool_errors ? <b style={{ color: 'var(--red)' }}> ({meta.tool_errors} 失败)</b> : null}</span>
            )}
            <span>{fmtTime(span.created_at)}</span>
          </div>
          {span.error && <TraceBlock label="错误" text={span.error} tone="error" />}
          {span.system_prompt && <TraceBlock label="系统提示" text={span.system_prompt} />}
          <TraceBlock label="入参 INPUT" text={span.input ?? ''} />
          <TraceBlock label="出参 OUTPUT" text={span.output ?? ''} />
        </div>
      )}
    </div>
  );
}

function TraceBlock({ label, text, tone }: { label: string; text: string; tone?: 'error' }) {
  if (!text) return null;
  return (
    <div>
      <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', letterSpacing: '.12em', textTransform: 'uppercase', color: 'var(--text-faint)', marginBottom: 4 }}>{label}</div>
      <pre style={{
        margin: 0, padding: '10px 12px', background: 'var(--code-bg)', borderRadius: 'var(--radius-sm)',
        border: '1px solid var(--border)', color: tone === 'error' ? 'var(--red)' : 'var(--text-body, var(--text))',
        fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', lineHeight: 'var(--leading-normal)',
        whiteSpace: 'pre-wrap', wordBreak: 'break-word', maxHeight: 360, overflow: 'auto',
      }}>{text}</pre>
    </div>
  );
}

// 产出解析状态 → 语义色。
const OUT_STATUS_CHIP: Record<string, string> = { ok: 'green', partial: 'amber', error: 'red' };

// 环节 Agent 结构化产出浏览器（agent_outputs）：流水线级粒度，点 trace 下钻到单步推理。
function AgentOutputsExplorer({ onDrill }: { onDrill: (traceId: string) => void }) {
  const [role, setRole] = useState('');
  const [targetId, setTargetId] = useState('');
  const [status, setStatus] = useState('');
  const [roles, setRoles] = useState<string[]>([]);
  const [rows, setRows] = useState<AgentOutputSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [detail, setDetail] = useState<AgentOutput | null>(null);
  const [copied, setCopied] = useState(false);

  const load = useCallback(() => {
    setLoading(true);
    listAgentOutputs({
      role: role || undefined,
      target_id: targetId.trim() || undefined,
      status: status || undefined,
      limit: 300,
    }).then(setRows).catch(() => setRows([])).finally(() => setLoading(false));
  }, [role, targetId, status]);

  useEffect(() => { listAgentOutputRoles().then(setRoles).catch(() => {}); }, []);
  useEffect(() => { const t = setTimeout(load, 300); return () => clearTimeout(t); }, [load]);

  const open = (id: string) => {
    setSelected(id);
    setDetail(null);
    getAgentOutput(id).then(setDetail).catch(() => setDetail(null));
  };

  const pretty = useMemo(() => {
    if (!detail?.output_json) return '';
    try { return JSON.stringify(JSON.parse(detail.output_json), null, 2); } catch { return detail.output_json; }
  }, [detail]);

  const copyOut = async () => {
    if (!pretty) return;
    try { await navigator.clipboard.writeText(pretty); setCopied(true); setTimeout(() => setCopied(false), 1500); } catch { /* ignore */ }
  };

  // 从结构化产出里取一句话结论：先认已知 schema 的关键字段，再退化为「首个非空短字符串叶子」兜底，
  // 让 proposer/doc_writer 等未枚举的环节产出在列表里也有可读摘要。
  const headline = (j: string | null): string => {
    if (!j) return '';
    try {
      const o = JSON.parse(j);
      if (o.verdict) return `${o.verdict}${o.summary ? ' · ' + o.summary : ''}`;
      if (o.triage?.analysis_summary) return o.triage.analysis_summary;
      if (o.summary) return o.summary;
      if (typeof o.title === 'string' && o.title.trim()) return o.title.trim();
      // 兜底：扫顶层字符串字段，取首个长度适中的非空值。
      for (const v of Object.values(o)) {
        if (typeof v === 'string' && v.trim().length >= 4 && v.length <= 200) return v.trim();
      }
      return '';
    } catch { return ''; }
  };

  return (
    <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
      {/* 左：筛选 + 产出列表 */}
      <div className="list-col" style={{ width: 360, flex: '0 0 360px', display: 'flex', flexDirection: 'column' }}>
        <div style={{ padding: '8px 10px', display: 'flex', flexDirection: 'column', gap: 6, borderBottom: '1px solid var(--border)' }}>
          <input className="trace-filter-input" value={targetId} onChange={e => setTargetId(e.target.value)} placeholder="🔍 关联编号（需求 / CR）" />
          <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
            <Select value={role} onChange={setRole} style={{ flex: 1, minWidth: 0 }}
              options={[{ value: '', label: '全部环节' }, ...roles.map(r => ({ value: r, label: r }))]} />
            <div className="seg" style={{ flexShrink: 0 }}>
              {[['', '全部'], ['ok', '成功'], ['partial', '部分'], ['error', '失败']].map(([v, l]) => (
                <button key={v} className={status === v ? 'on' : ''} onClick={() => setStatus(v)}
                  title={v === 'partial' ? '部分：批量环节（如 proposer）返回的 JSON 数组里，部分条目解析成功、部分损坏（坏条目已逐条跳过，保留好的）。单产出环节（如 analysis）不会出现此状态。' : undefined}>{l}</button>
              ))}
            </div>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-faint)', padding: '1px 2px' }}>
            <span>{loading ? '查询中…' : `共 ${rows.length} 条`}</span>
          </div>
        </div>
        <div className="list-body scroll" style={{ flex: 1 }}>
          {!loading && rows.length === 0 && (
            <div className="empty" style={{ padding: 24, textAlign: 'center', color: 'var(--text-3)' }}>
              <Icon name="log" size={28} /><div style={{ marginTop: 8 }}>暂无环节产出</div>
            </div>
          )}
          {rows.map(r => (
            <div key={r.id} className={'cfg-card selectable' + (selected === r.id ? ' picked' : '')} onClick={() => open(r.id)}
              style={{ margin: '8px 10px', padding: '10px 12px' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <span className="chip ember" style={{ fontSize: 'var(--text-micro)' }}>{r.role}</span>
                <span className={'chip ' + (OUT_STATUS_CHIP[r.status] ?? '')} style={{ fontSize: 'var(--text-micro)' }}>{r.status}</span>
                <span style={{ marginLeft: 'auto', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-3)' }}>v{r.schema_version}</span>
              </div>
              <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-2)', margin: '6px 0', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {preview(headline(r.output_json), 64) || '—'}
              </div>
              <div style={{ display: 'flex', gap: 10, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-faint)' }}>
                <span>{r.target_kind}:{preview(r.target_id, 14)}</span>
                {r.trace_id && <span title="可下钻到调用链"><Icon name="log" size={10} /> trace</span>}
                <span style={{ marginLeft: 'auto' }}>{fmtTime(r.created_at)}</span>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* 右：选中产出详情 */}
      <div className="scroll" style={{ flex: 1, minWidth: 0, padding: '16px 20px', overflow: 'auto' }}>
        {!selected && (
          <div className="empty" style={{ marginTop: 80, textAlign: 'center', color: 'var(--text-3)' }}>
            <Icon name="eye" size={32} /><div style={{ marginTop: 10 }}>从左侧选择一条环节产出查看结构化结果</div>
          </div>
        )}
        {selected && !detail && <div className="empty" style={{ padding: 24 }}>加载中…</div>}
        {selected && detail && (
          <>
            <div className="panel" style={{ marginBottom: 12 }}>
              <div className="panel-head">
                <div className="panel-title"><Icon name="layers" size={16} style={{ color: 'var(--ember)' }} />{detail.role} · 结构化产出</div>
                <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
                  {detail.trace_id && (
                    <button className="btn btn-sm" onClick={() => onDrill(detail.trace_id!)}
                      title="下钻到本次调用的 LLM/工具 调用链">
                      <Icon name="log" size={13} />查看调用链
                    </button>
                  )}
                  <button className="btn btn-sm" onClick={copyOut} title="复制结构化产出 JSON">
                    <Icon name={copied ? 'check' : 'copy'} size={13} />{copied ? '已复制' : '复制 JSON'}
                  </button>
                </div>
              </div>
              <div style={{ padding: '12px 14px', display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '8px 18px', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>
                <span>环节：<b style={{ color: 'var(--text-2)' }}>{detail.role}</b></span>
                <span>schema：<b style={{ color: 'var(--text-2)' }}>v{detail.schema_version}</b></span>
                <span>状态：<b style={{ color: 'var(--text-2)' }}>{detail.status}</b></span>
                <span>关联：<b style={{ color: 'var(--text-2)' }}>{detail.target_kind}:{detail.target_id}</b></span>
                <span>项目：<b style={{ color: 'var(--text-2)' }}>{detail.project_id || '—'}</b></span>
                <span>trace：<b style={{ color: 'var(--text-2)' }}>{detail.trace_id ? detail.trace_id.slice(0, 8) : '—'}</b></span>
              </div>
            </div>
            <TraceBlock label="结构化产出 OUTPUT" text={pretty} />
            {detail.raw && <TraceBlock label="原始模型文本 RAW" text={detail.raw} />}
          </>
        )}
      </div>
    </div>
  );
}

// schema 体检：选 role（+ 可选 schema 版本）→ 字段填充率 + 状态分布，暴露长期空着的弱字段（优化循环）。
// 体检锚定单一 schema 版本（默认最新），避免跨版本字段漂移污染填充率。
function SchemaHealth() {
  const [roles, setRoles] = useState<string[]>([]);
  const [role, setRole] = useState('');
  const [version, setVersion] = useState('');  // 空 = 后端解析为最新版本
  const [data, setData] = useState<FieldHealth | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    listAgentOutputRoles().then(rs => {
      setRoles(rs);
      setRole(prev => prev || rs[0] || '');
    }).catch(() => {});
  }, []);

  // 切换环节时重置版本选择，让后端回到「最新版本」默认。
  useEffect(() => { setVersion(''); }, [role]);

  useEffect(() => {
    if (!role) { setData(null); return; }
    setLoading(true);
    agentOutputFieldHealth(role, version || undefined)
      .then(setData).catch(() => setData(null)).finally(() => setLoading(false));
  }, [role, version]);

  const pct = (n: number) => `${Math.round(n * 100)}%`;
  // 填充率 → 语义色：低=红（弱字段）、中=琥珀、高=绿。
  const barColor = (r: number) => r < 0.34 ? 'var(--red)' : r < 0.67 ? 'var(--amber)' : 'var(--green)';

  return (
    <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
      <div className="list-col" style={{ width: 220, flex: '0 0 220px', display: 'flex', flexDirection: 'column' }}>
        <div style={{ padding: '8px 10px', borderBottom: '1px solid var(--border)', display: 'flex', flexDirection: 'column', gap: 10 }}>
          <div>
            <div className="eyebrow" style={{ fontSize: 'var(--text-caption)', marginBottom: 6 }}><span className="en">ROLE</span></div>
            <Select value={role} onChange={setRole} style={{ width: '100%' }}
              options={roles.map(r => ({ value: r, label: r }))} placeholder="选择环节" />
          </div>
          {data && data.versions.length > 0 && (
            <div>
              <div className="eyebrow" style={{ fontSize: 'var(--text-caption)', marginBottom: 6 }}><span className="en">SCHEMA</span></div>
              <Select value={data.schema_version ?? ''} onChange={setVersion} style={{ width: '100%' }}
                options={data.versions.map(v => ({ value: v.schema_version, label: `v${v.schema_version} · ${v.count}` }))}
                placeholder="版本" />
            </div>
          )}
        </div>
      </div>
      <div style={{ flex: 1, minWidth: 0, overflow: 'auto', padding: 18 }}>
        {!role ? (
          <div className="empty"><Icon name="log" size={32} /><div style={{ marginTop: 10 }}>暂无可体检的环节产出</div></div>
        ) : loading ? (
          <div className="empty"><div>体检中…</div></div>
        ) : !data || data.total === 0 ? (
          <div className="empty"><Icon name="log" size={28} /><div style={{ marginTop: 8 }}>该环节暂无产出样本</div></div>
        ) : (
          <>
            <div className="panel" style={{ marginBottom: 14 }}>
              <div className="panel-head"><span>样本与解析状态 · {data.role} v{data.schema_version ?? '—'}</span></div>
              <div style={{ display: 'flex', gap: 8, padding: 14, flexWrap: 'wrap' }}>
                <span className="chip">样本 {data.total}</span>
                <span className="chip green">ok {data.status_ok}</span>
                <span className="chip amber" title="部分：批量环节（如 proposer）返回的 JSON 数组里，部分条目解析成功、部分损坏（坏条目已逐条跳过，保留好的）。单产出环节（如 analysis）不会出现此状态。">partial {data.status_partial}</span>
                <span className="chip red">error {data.status_error}</span>
              </div>
            </div>
            <div className="panel">
              <div className="panel-head"><span>字段填充率（升序 · 越靠前越该关注）</span></div>
              <div style={{ padding: 14, display: 'flex', flexDirection: 'column', gap: 8 }}>
                {data.fields.map(f => (
                  <div key={f.path} style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                    <code style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-2)', flex: '0 0 200px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{f.path}</code>
                    <div style={{ flex: 1, height: 8, background: 'var(--bg-3)', borderRadius: 'var(--radius-sm)', overflow: 'hidden' }}>
                      <div style={{ width: pct(f.fill_rate), height: '100%', background: barColor(f.fill_rate) }} />
                    </div>
                    <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-3)', flex: '0 0 96px', textAlign: 'right', whiteSpace: 'nowrap' }}>{pct(f.fill_rate)} · {f.filled}/{f.total}</span>
                  </div>
                ))}
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

// 用量 Tab：按模型聚合的 token 消耗与调用次数 + 合计。时间范围可切「近 7 天 / 30 天 / 全部」。
const USAGE_RANGES: { label: string; days: number }[] = [
  { label: '近 7 天', days: 7 },
  { label: '近 30 天', days: 30 },
  { label: '全部', days: 0 },
];
function UsageTab() {
  const [days, setDays] = useState(7);
  const [stats, setStats] = useState<LlmUsageStats | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    const since = days > 0
      ? new Date(Date.now() - days * 86400000).toISOString().slice(0, 19).replace('T', ' ')
      : undefined;
    llmUsageStats(since)
      .then(s => { if (alive) setStats(s); })
      .catch(() => { if (alive) setStats(null); })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [days]);

  const fmt = (n: number) => n.toLocaleString();
  return (
    <div className="scroll" style={{ flex: 1, padding: 'clamp(12px, 1.6vw, 24px)' }}>
      <div className="seg" style={{ marginBottom: 16, width: 'fit-content' }}>
        {USAGE_RANGES.map(r => (
          <button key={r.days} className={days === r.days ? 'on' : ''} onClick={() => setDays(r.days)}>{r.label}</button>
        ))}
      </div>

      {/* 合计卡 */}
      <div className="stat-grid" style={{ marginBottom: 16 }}>
        {[
          { label: '调用次数', val: stats ? fmt(stats.total_calls) : '—', color: 'var(--blue)' },
          { label: '输入 tokens', val: stats ? fmt(stats.total_prompt_tokens) : '—', color: 'var(--violet)' },
          { label: '输出 tokens', val: stats ? fmt(stats.total_completion_tokens) : '—', color: 'var(--amber)' },
          { label: '总 tokens', val: stats ? fmt(stats.total_tokens) : '—', color: 'var(--ember)' },
        ].map((c, i) => (
          <div className="stat" key={i}>
            <div className="stat-main">
              <div className="stat-label">{c.label}</div>
              <div className="stat-val" style={{ color: c.color }}>{c.val}</div>
            </div>
          </div>
        ))}
      </div>

      {/* 按模型明细表 */}
      <div className="panel">
        <div className="panel-head"><span style={{ fontWeight: 700 }}>按模型用量</span></div>
        <div style={{ padding: '8px 0' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 'var(--text-control)' }}>
            <thead>
              <tr style={{ color: 'var(--text-3)', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', textTransform: 'uppercase' }}>
                <th style={{ textAlign: 'left', padding: '6px 14px' }}>模型</th>
                <th style={{ textAlign: 'right', padding: '6px 14px' }}>调用</th>
                <th style={{ textAlign: 'right', padding: '6px 14px' }}>输入</th>
                <th style={{ textAlign: 'right', padding: '6px 14px' }}>输出</th>
                <th style={{ textAlign: 'right', padding: '6px 14px' }}>总计</th>
              </tr>
            </thead>
            <tbody>
              {stats?.rows.map((r, i) => (
                <tr key={i} style={{ borderTop: '1px solid var(--border)' }}>
                  <td style={{ padding: '8px 14px' }}>
                    <span style={{ fontFamily: 'var(--font-mono)' }}>{r.model || '—'}</span>
                    {r.provider && <span className="chip" style={{ marginLeft: 6, fontSize: 'var(--text-micro)' }}>{r.provider}</span>}
                  </td>
                  <td style={{ textAlign: 'right', padding: '8px 14px' }}>{fmt(r.calls)}</td>
                  <td style={{ textAlign: 'right', padding: '8px 14px', color: 'var(--text-3)' }}>{fmt(r.prompt_tokens)}</td>
                  <td style={{ textAlign: 'right', padding: '8px 14px', color: 'var(--text-3)' }}>{fmt(r.completion_tokens)}</td>
                  <td style={{ textAlign: 'right', padding: '8px 14px', fontWeight: 600 }}>{fmt(r.total_tokens)}</td>
                </tr>
              ))}
              {!loading && (!stats || stats.rows.length === 0) && (
                <tr><td colSpan={5} style={{ padding: '20px 14px', textAlign: 'center', color: 'var(--text-faint)' }}>暂无用量数据</td></tr>
              )}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}

export default function TracePage() {
  const [tab, setTab] = useState<'trace' | 'outputs' | 'usage' | 'health'>('trace');
  const [usageKey, setUsageKey] = useState(0);
  const [outputsKey, setOutputsKey] = useState(0);
  const [healthKey, setHealthKey] = useState(0);
  const [issueId, setIssueId] = useState('');
  const [conversationId, setConversationId] = useState('');
  const [agentName, setAgentName] = useState('');
  const [status, setStatus] = useState('');
  const [keyword, setKeyword] = useState('');

  const [agentNames, setAgentNames] = useState<string[]>([]);
  const [traces, setTraces] = useState<LlmTraceSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [spans, setSpans] = useState<LlmTrace[]>([]);
  const [spansLoading, setSpansLoading] = useState(false);
  const [copied, setCopied] = useState(false);

  const copyTrace = async () => {
    const text = buildTraceText(spans);
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch { /* clipboard unavailable — ignore */ }
  };

  const load = useCallback(() => {
    setLoading(true);
    const filter: TraceFilter = {
      issue_id: issueId.trim() || undefined,
      conversation_id: conversationId.trim() || undefined,
      agent_name: agentName || undefined,
      status: status || undefined,
      kind: keyword.trim() || undefined,
      limit: 300,
    };
    listLlmTraces(filter).then(setTraces).catch(() => setTraces([])).finally(() => setLoading(false));
  }, [issueId, conversationId, agentName, status, keyword]);

  useEffect(() => { listTraceAgentNames().then(setAgentNames).catch(() => {}); }, []);
  // 输入防抖：筛选条件变化 300ms 后查询。
  useEffect(() => { const t = setTimeout(load, 300); return () => clearTimeout(t); }, [load]);

  const openTrace = (id: string) => {
    setSelected(id);
    setSpansLoading(true);
    getLlmTrace(id).then(setSpans).catch(() => setSpans([])).finally(() => setSpansLoading(false));
  };

  const root = spans.find(s => s.kind === 'agent') ?? spans[0];

  return (
    <div className="content">
      <div className="audit-top" style={{ height: 56 }}>
        <div className="eyebrow" style={{ fontSize: 'var(--text-heading)' }}>
          <span className="en">TRACE</span><span className="cn">· 智能体可观测</span>
        </div>
        <div className="seg" style={{ marginLeft: 16 }}>
          <button className={tab === 'trace' ? 'on' : ''} onClick={() => setTab('trace')}>调用链路</button>
          <button className={tab === 'outputs' ? 'on' : ''} onClick={() => setTab('outputs')}>环节产出</button>
          <button className={tab === 'usage' ? 'on' : ''} onClick={() => setTab('usage')}>用量</button>
          <button className={tab === 'health' ? 'on' : ''} onClick={() => setTab('health')}>schema 体检</button>
        </div>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
          {tab === 'trace' ? (
            <>
              <button className="btn btn-sm" onClick={load} title="刷新"><Icon name="refresh" size={14} />刷新</button>
              <button className="btn btn-sm btn-danger" onClick={() => { if (confirm('确认清空全部 trace 记录？')) clearLlmTraces().then(() => { setTraces([]); setSpans([]); setSelected(null); }); }}>
                <Icon name="trash" size={14} />清空
              </button>
            </>
          ) : tab === 'outputs' ? (
            <>
              <button className="btn btn-sm" onClick={() => setOutputsKey(k => k + 1)} title="刷新"><Icon name="refresh" size={14} />刷新</button>
              <button className="btn btn-sm btn-danger" onClick={() => { if (confirm('确认清空全部环节产出记录？')) clearAgentOutputs().then(() => setOutputsKey(k => k + 1)); }}>
                <Icon name="trash" size={14} />清空
              </button>
            </>
          ) : tab === 'usage' ? (
            <button className="btn btn-sm" onClick={() => setUsageKey(k => k + 1)} title="刷新"><Icon name="refresh" size={14} />刷新</button>
          ) : (
            <button className="btn btn-sm" onClick={() => setHealthKey(k => k + 1)} title="刷新"><Icon name="refresh" size={14} />刷新</button>
          )}
        </div>
      </div>

      {tab === 'outputs' && (
        <AgentOutputsExplorer key={outputsKey} onDrill={(tid) => { setTab('trace'); openTrace(tid); }} />
      )}

      {tab === 'usage' && <UsageTab key={usageKey} />}

      {tab === 'health' && <SchemaHealth key={healthKey} />}

      {tab === 'trace' && (
      <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
        {/* 左：筛选 + trace 列表 */}
        <div className="list-col" style={{ width: 360, flex: '0 0 360px', display: 'flex', flexDirection: 'column' }}>
          <div style={{ padding: '8px 10px', display: 'flex', flexDirection: 'column', gap: 6, borderBottom: '1px solid var(--border)' }}>
            {/* 主搜索：关键词命中 输入/输出/名称 */}
            <input className="trace-filter-input" value={keyword} onChange={e => setKeyword(e.target.value)}
              placeholder="🔍 搜索 输入 / 输出 / 名称…" />
            {/* 需求编号 + 会议室编号 并排 */}
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6 }}>
              <input className="trace-filter-input" value={issueId} onChange={e => setIssueId(e.target.value)} placeholder="需求编号" />
              <input className="trace-filter-input" value={conversationId} onChange={e => setConversationId(e.target.value)} placeholder="会议室编号" />
            </div>
            {/* 角色下拉 + 状态分段 并排 */}
            <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
              <Select value={agentName} onChange={setAgentName} style={{ flex: 1, minWidth: 0 }}
                options={[{ value: '', label: '全部角色' }, ...agentNames.map(r => ({ value: r, label: r }))]} />
              <div className="seg" style={{ flexShrink: 0 }}>
                {[['', '全部'], ['ok', '成功'], ['error', '失败']].map(([v, l]) => (
                  <button key={v} className={status === v ? 'on' : ''} onClick={() => setStatus(v)}>{l}</button>
                ))}
              </div>
            </div>
            {/* 结果计数 + 一键重置（有任意筛选时显示） */}
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-faint)', padding: '1px 2px' }}>
              <span>{loading ? '查询中…' : `共 ${traces.length} 条`}</span>
              {(keyword || issueId || conversationId || agentName || status) && (
                <button className="btn btn-ghost btn-sm" style={{ padding: '1px 6px', height: 'auto', fontSize: 'var(--text-micro)', gap: 3 }}
                  onClick={() => { setKeyword(''); setIssueId(''); setConversationId(''); setAgentName(''); setStatus(''); }}>
                  <Icon name="x" size={11} />重置
                </button>
              )}
            </div>
          </div>
          <div className="list-body scroll" style={{ flex: 1 }}>
            {loading && <div className="empty" style={{ padding: 24 }}>加载中…</div>}
            {!loading && traces.length === 0 && (
              <div className="empty" style={{ padding: 24, textAlign: 'center', color: 'var(--text-3)' }}>
                <Icon name="log" size={28} /><div style={{ marginTop: 8 }}>暂无 trace 记录</div>
              </div>
            )}
            {traces.map(t => (
              <div key={t.trace_id}
                className={'cfg-card selectable' + (selected === t.trace_id ? ' picked' : '')}
                onClick={() => openTrace(t.trace_id)}
                style={{ margin: '8px 10px', padding: '10px 12px' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <span className="chip ember" style={{ fontSize: 'var(--text-micro)' }}>{t.agent_name || t.agent_role || 'agent'}</span>
                  {t.status === 'error' && <span className="chip red" style={{ fontSize: 'var(--text-micro)' }}>error</span>}
                  {t.innate_triggered && <InnateChip />}
                  <span style={{ marginLeft: 'auto', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-3)' }}>{fmtMs(t.latency_ms)}</span>
                </div>
                <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-2)', margin: '6px 0', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {t.agent_name || '—'}：{preview(t.input, 60)}
                </div>
                <div style={{ display: 'flex', gap: 10, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-faint)' }}>
                  <span>{t.span_count} spans</span>
                  {t.total_tokens != null && <span>{t.total_tokens} tokens</span>}
                  <span style={{ marginLeft: 'auto' }}>{fmtTime(t.created_at)}</span>
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* 右：选中 trace 的 span 明细 */}
        <div className="scroll" style={{ flex: 1, minWidth: 0, padding: '16px 20px', overflow: 'auto' }}>
          {!selected && (
            <div className="empty" style={{ marginTop: 80, textAlign: 'center', color: 'var(--text-3)' }}>
              <Icon name="eye" size={32} /><div style={{ marginTop: 10 }}>从左侧选择一条 trace 查看完整调用链</div>
            </div>
          )}
          {selected && spansLoading && <div className="empty" style={{ padding: 24 }}>加载中…</div>}
          {selected && !spansLoading && root && (
            <>
              <div className="panel" style={{ marginBottom: 12 }}>
                <div className="panel-head">
                  <div className="panel-title"><Icon name="log" size={16} style={{ color: 'var(--ember)' }} />Trace 概览</div>
                  {spans.some(s => s.innate_triggered) && <span style={{ marginLeft: 8 }}><InnateChip size={11} /></span>}
                  <button className="btn btn-sm" style={{ marginLeft: 'auto' }} onClick={copyTrace}
                    title="复制完整 trace（Markdown），用于粘贴给 Claude 等工具分析优化">
                    <Icon name={copied ? 'check' : 'copy'} size={13} />{copied ? '已复制' : '复制完整 trace'}
                  </button>
                </div>
                <div style={{ padding: '12px 14px', display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '8px 18px', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>
                  <span>角色：<b style={{ color: 'var(--text-2)' }}>{root.agent_name || '—'}</b></span>
                  <span title={root.agent_role || ''} style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>职责：<b style={{ color: 'var(--text-2)' }}>{root.agent_role || '—'}</b></span>
                  <span>需求：<b style={{ color: 'var(--text-2)' }}>{root.issue_id || '—'}</b></span>
                  <span>会议室：<b style={{ color: 'var(--text-2)' }}>{root.conversation_id || '—'}</b></span>
                  <span>项目：<b style={{ color: 'var(--text-2)' }}>{root.project_id || '—'}</b></span>
                  <span>任务：<b style={{ color: 'var(--text-2)' }}>{root.task_id || '—'}</b></span>
                  <span>总耗时：<b style={{ color: 'var(--text-2)' }}>{fmtMs(root.latency_ms)}</b></span>
                  <span>span 数：<b style={{ color: 'var(--text-2)' }}>{spans.length}</b></span>
                </div>
              </div>
              {spans.map(s => <SpanRow key={s.id} span={s} />)}
            </>
          )}
        </div>
      </div>
      )}
    </div>
  );
}
