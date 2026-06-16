import React, { useState, useEffect, useCallback, useMemo } from 'react';
import Icon from '../components/Icon';
import Select from '../components/Select';
import {
  listLlmTraces, getLlmTrace, listTraceAgentNames, clearLlmTraces,
  type LlmTraceSummary, type LlmTrace, type TraceFilter,
} from '../services';

// span 类型 → chip 语义色（仅语义状态色，遵循设计系统）。
const KIND_CHIP: Record<string, string> = {
  agent: 'ember', llm: 'blue', tool: 'violet', mcp: 'green',
};
const KIND_LABEL: Record<string, string> = {
  agent: 'AGENT', llm: 'LLM', tool: 'TOOL', mcp: 'MCP',
};

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
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexShrink: 0, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-3)' }}>
          {span.total_tokens != null && <span title="tokens">{span.total_tokens} tok</span>}
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

export default function TracePage() {
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
          <span className="en">LLM TRACE</span><span className="cn">· 调用链路追踪</span>
        </div>
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 8 }}>
          <button className="btn btn-sm" onClick={load} title="刷新"><Icon name="refresh" size={14} />刷新</button>
          <button className="btn btn-sm btn-danger" onClick={() => { if (confirm('确认清空全部 trace 记录？')) clearLlmTraces().then(() => { setTraces([]); setSpans([]); setSelected(null); }); }}>
            <Icon name="trash" size={14} />清空
          </button>
        </div>
      </div>

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
                className="cfg-card"
                onClick={() => openTrace(t.trace_id)}
                style={{ margin: '8px 10px', padding: '10px 12px', cursor: 'pointer', ...(selected === t.trace_id ? { borderColor: 'var(--ember-tint-strong)' } : {}) }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <span className="chip ember" style={{ fontSize: 'var(--text-micro)' }}>{t.agent_name || t.agent_role || 'agent'}</span>
                  {t.status === 'error' && <span className="chip red" style={{ fontSize: 'var(--text-micro)' }}>error</span>}
                  <span style={{ marginLeft: 'auto', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-3)' }}>{fmtMs(t.latency_ms)}</span>
                </div>
                <div style={{ fontSize: 'var(--text-label)', color: 'var(--text-2)', margin: '6px 0', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {t.agent_name || '—'}：{preview(t.input, 60)}
                </div>
                <div style={{ display: 'flex', gap: 10, fontFamily: 'var(--font-mono)', fontSize: 'var(--text-micro)', color: 'var(--text-faint)' }}>
                  <span>{t.span_count} spans</span>
                  {t.total_tokens != null && <span>{t.total_tokens} tok</span>}
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
    </div>
  );
}
