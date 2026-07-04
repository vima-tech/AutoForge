import React, { useEffect, useMemo, useRef, useState } from 'react';
import Icon from './Icon';
import Select from './Select';
import { listContextItems, type ContextItem } from '../services';

/**
 * 全量上下文基质 · 内联引用选择器（《全量上下文基质·万物可引》契约 §7 + 搜索优先取用方案）。
 *
 * 一个组件吃掉所有功能点的「引用上下文」：会议室 @、孵化台起草、需求提交、审核建议…
 * 复用 `mention-pop / mention-row` 弹层 + `Select` 来源筛选，底层调 `list_context_items`
 * 活查全量来源（需求/会议/日志/trace/规格/.autoforge 文件…）。**不做角色视角，只按来源筛。**
 *
 * 交互形态（搜索优先）：**空弹层是入口不是清单**——默认态只展示分组候选面板
 * （产物组展开、过程/配置/外部折叠成计数组头）；输入关键词即调后端全量召回
 * （LIKE 下推 provider，可及范围=全部来源，不受默认拉取条数限制）。
 *
 * 调用方通过 `onPick(item)` 决定落法：会议室落 `context_ref` 块、文本面板内联引用段等。
 */

/** 来源类型人可读名（与后端 `core::context::source_kind` 对齐；未知回落原值）。 */
export const KIND_LABELS: Record<string, string> = {
  issue: '需求',
  incubator_draft: '孵化台草稿',
  project_spec: '项目规格',
  code_agent_log: '编码日志',
  llm_trace: 'Agent Trace',
  chat_message: '会议室消息',
  agent_output: 'Agent 输出',
  cr_review: '审核意见',
  security_audit: '安全审计',
  test_session: '测试会话',
  scan_finding: '扫描发现',
  deployment: '部署',
  delivery_artifact: '交付产物',
  worktree_session: 'Worktree 会话',
  prototype_prompt: '原型提示',
  material: '物料',
  attachment: '附件',
  workspace_doc: '文档(.autoforge)',
  workspace_spec: '规格(.autoforge)',
  workspace_deliverable: '交付(.autoforge)',
  project_meta: '项目指引',
  cfg_agent: 'Agent 配置',
  cfg_code_agent: '编码Agent 配置',
  cfg_mcp: 'MCP 配置',
  file_priority: 'claude.md/agents.md',
  file_pinned: 'Pinned 文件',
};

export const kindLabel = (k: string) => KIND_LABELS[k] ?? k;

/**
 * 呈现分组（纯 UI 概念；注入优先级见后端 `core/context.rs::kind_rank`，两者独立维护）。
 * 默认态「产物」展开、其余折叠成计数组头——26 类平铺是操作困扰的根源，分组即抽屉。
 */
const KIND_GROUPS: Record<string, string[]> = {
  产物: [
    'file_pinned', 'file_priority', 'project_spec', 'workspace_spec', 'workspace_doc',
    'workspace_deliverable', 'material', 'issue', 'incubator_draft', 'delivery_artifact',
    'prototype_prompt', 'project_meta', 'attachment',
  ],
  过程: [
    'chat_message', 'agent_output', 'code_agent_log', 'llm_trace', 'cr_review',
    'test_session', 'scan_finding', 'deployment', 'worktree_session', 'security_audit',
  ],
  配置: ['cfg_agent', 'cfg_code_agent', 'cfg_mcp'],
  外部: ['mcp_result', 'web_result'],
};
const GROUP_ORDER = ['产物', '过程', '配置', '外部'] as const;
/** kind → 组名（未知 kind 兜底进「过程」，保证新来源不消失）。 */
const groupOf = (kind: string): string => {
  for (const g of GROUP_ORDER) if (KIND_GROUPS[g].includes(kind)) return g;
  return '过程';
};
/** 默认态每 kind 最多展示条数（产物组）；折叠组展开后的组内上限。 */
const PER_KIND_DEFAULT = 3;
const PER_GROUP_EXPANDED = 10;

/** 产生阶段人可读名（与后端 `origin_stage` 对齐）。 */
const STAGE_LABELS: Record<string, string> = {
  requirement: '需求', design: '设计', chat: '会议', coding: '编码', review: '审核', ops: '运维',
};
const stageLabel = (s: string) => STAGE_LABELS[s] ?? s;

/** 相对时间（刚刚 / N分钟前 / N小时前 / N天前 / 日期）。入参为后端 UTC "YYYY-MM-DD HH:MM:SS"。 */
function relTime(raw: string): string {
  if (!raw) return '';
  // 后端时间是 UTC 但无时区后缀，补 'Z' 让 Date 正确解析。
  const iso = raw.includes('T') ? raw : raw.replace(' ', 'T') + (raw.endsWith('Z') ? '' : 'Z');
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '';
  const diff = Date.now() - t;
  const m = Math.floor(diff / 60000), h = Math.floor(diff / 3600000), d = Math.floor(diff / 86400000);
  if (diff < 60000) return '刚刚';
  if (m < 60) return `${m}分钟前`;
  if (h < 24) return `${h}小时前`;
  if (d < 30) return `${d}天前`;
  return new Date(t).toLocaleDateString();
}

/** 体积（B / KB / MB）。 */
function formatBytes(n: number): string {
  if (!n || n <= 0) return '';
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / 1048576).toFixed(1)}MB`;
}

type Placement = 'up' | 'down';

interface Props {
  projectId: string;
  onPick: (item: ContextItem) => void;
  /** 限定可选来源（空=全部）。 */
  sourceKinds?: string[];
  /** 弹层展开方向（默认向上，适配底部输入框）。 */
  placement?: Placement;
  /** 触发按钮内容（默认「引用」）。 */
  trigger?: React.ReactNode;
  triggerClassName?: string;
  title?: string;
  disabled?: boolean;
}

export default function ContextPicker({
  projectId, onPick, sourceKinds, placement = 'up', trigger, triggerClassName, title, disabled,
}: Props) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  // 防抖后的搜索词（200ms）：真正下发后端的 query。
  const [serverQuery, setServerQuery] = useState('');
  const [kind, setKind] = useState('');
  const [items, setItems] = useState<ContextItem[]>([]);
  // 全部可选来源（由未筛选那次加载捕获，独立于当前筛选，避免选中某来源后其余选项消失）。
  const [allKinds, setAllKinds] = useState<string[]>([]);
  // 默认态展开的组（产物常开；其余点组头展开）。
  const [openGroups, setOpenGroups] = useState<Set<string>>(() => new Set(['产物']));
  const [loading, setLoading] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  // 输入防抖 → serverQuery。
  useEffect(() => {
    const t = setTimeout(() => setServerQuery(query.trim()), 200);
    return () => clearTimeout(t);
  }, [query]);

  const searching = serverQuery.length > 0;

  // 打开/搜索词/来源变化时拉取。搜索态走后端全量召回（LIKE 下推），默认态拉分组候选。
  useEffect(() => {
    if (!open || !projectId) return;
    setLoading(true);
    const restricted = !!(sourceKinds && sourceKinds.length);
    const kinds = kind ? [kind] : (restricted ? sourceKinds : undefined);
    listContextItems(projectId, kinds, searching ? 100 : 120, searching ? serverQuery : undefined)
      .then(list => {
        setItems(list);
        // 仅在「未按 kind 筛选、非搜索」的加载上刷新完整来源集合，筛选后不收缩下拉选项。
        if (!kind && !restricted && !searching) setAllKinds(Array.from(new Set(list.map(i => i.source_kind))));
      })
      .catch(() => setItems([]))
      .finally(() => setLoading(false));
  }, [open, projectId, kind, sourceKinds, searching, serverQuery]);

  // 点击外部关闭。
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as HTMLElement | null;
      // 来源下拉（Select）弹层经 portal 挂到 document.body，落在 wrapRef 之外；
      // 点它的选项不算「点击外部」，否则选来源就会连带关掉整个引用面板。
      if (t?.closest?.('.csel-pop')) return;
      if (wrapRef.current && !wrapRef.current.contains(t as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);

  // 可选来源下拉：限定集合优先，否则用未筛选加载捕获的完整来源集合（选中后不缩水）。
  const kindOptions = useMemo(() => {
    const base = sourceKinds && sourceKinds.length ? sourceKinds : allKinds;
    const opts = base.map(k => ({ value: k, label: kindLabel(k) }));
    return [{ value: '', label: '全部来源' }, ...opts];
  }, [allKinds, sourceKinds]);

  // 可搜索来源总数（底部提示用）：限定集合 > 已知全集 > 标签表兜底。
  const searchableCount =
    (sourceKinds && sourceKinds.length) || allKinds.length || Object.keys(KIND_LABELS).length;

  /**
   * 分组视图数据：组 → 该组条目（默认态产物组每 kind 截 PER_KIND_DEFAULT 条，
   * 折叠组展开后截 PER_GROUP_EXPANDED 条；搜索态全展开不截——后端已限量+排序）。
   * 单来源筛选（kind 有值）时退化为平铺列表，不走分组。
   */
  const grouped = useMemo(() => {
    const by = new Map<string, ContextItem[]>();
    for (const it of items) {
      const g = groupOf(it.source_kind);
      if (!by.has(g)) by.set(g, []);
      by.get(g)!.push(it);
    }
    return GROUP_ORDER
      .filter(g => by.has(g))
      .map(g => {
        const all = by.get(g)!;
        let show = all;
        if (!searching) {
          if (g === '产物' || openGroups.has(g)) {
            if (g === '产物') {
              // 每 kind 截前 N 条（后端已时间倒序，保留最近的）。
              const perKind = new Map<string, number>();
              show = all.filter(it => {
                const n = (perKind.get(it.source_kind) ?? 0) + 1;
                perKind.set(it.source_kind, n);
                return n <= PER_KIND_DEFAULT;
              });
            } else {
              show = all.slice(0, PER_GROUP_EXPANDED);
            }
          } else {
            show = [];
          }
        }
        return { group: g, total: all.length, show, expanded: searching || g === '产物' || openGroups.has(g) };
      });
  }, [items, searching, openGroups]);

  const pick = (it: ContextItem) => { onPick(it); setOpen(false); setQuery(''); setServerQuery(''); };
  const toggleGroup = (g: string) =>
    setOpenGroups(prev => {
      const next = new Set(prev);
      if (next.has(g)) next.delete(g); else next.add(g);
      return next;
    });

  const popStyle: React.CSSProperties = {
    left: 0, right: 'auto', width: 340, maxHeight: 380, overflowY: 'auto', zIndex: 80,
    ...(placement === 'up'
      ? { bottom: 'calc(100% + 6px)', top: 'auto', marginBottom: 0 }
      : { top: 'calc(100% + 6px)', bottom: 'auto', marginTop: 0 }),
  };

  const renderRow = (it: ContextItem) => {
    // 元信息行：相对时间 · 阶段 · 体积 · 预览片段（各段有值才显示）。
    const meta = [relTime(it.created_at), stageLabel(it.origin_stage), formatBytes(it.size_hint)]
      .filter(Boolean).join(' · ');
    return (
      <div key={it.id} className="mention-row" onClick={() => pick(it)} style={{ alignItems: 'flex-start' }}>
        <span className="chip" style={{ flexShrink: 0, marginTop: 1 }}>{kindLabel(it.source_kind)}</span>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div className="nm" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {it.title || it.id}
          </div>
          {(meta || it.preview) && (
            <div
              className="rl"
              style={{
                fontFamily: 'var(--font-mono)', overflow: 'hidden',
                textOverflow: 'ellipsis', whiteSpace: 'nowrap', marginTop: 1,
              }}
            >
              {meta}
              {it.preview && (
                <span style={{ color: 'var(--text-faint)' }}>{meta ? ' · ' : ''}{it.preview}</span>
              )}
            </div>
          )}
        </div>
        {it.trust === 'external_untrusted' && <span className="chip amber" style={{ flexShrink: 0, marginTop: 1 }}>外部</span>}
      </div>
    );
  };

  return (
    <div style={{ position: 'relative', display: 'inline-flex' }} ref={wrapRef}>
      <button
        type="button"
        className={triggerClassName || 'icon-btn'}
        title={title || '引用上下文（系统全量信息可引）'}
        disabled={disabled || !projectId}
        onClick={() => setOpen(o => !o)}
      >
        {trigger ?? <Icon name="layers" size={16} />}
      </button>
      {open && (
        <div className="mention-pop" style={popStyle}>
          <div className="mention-pop-label" style={{ display: 'block', padding: '6px 10px' }}>
            引用上下文 · 系统全量信息
          </div>
          <div style={{ padding: '4px 8px', display: 'flex', gap: 6 }}>
            <div style={{ flex: '0 0 130px' }}>
              <Select className="sm" value={kind} onChange={setKind} options={kindOptions} placeholder="来源" />
            </div>
            <input
              value={query}
              onChange={e => setQuery(e.target.value)}
              placeholder="🔍 搜索全部来源…"
              autoFocus
              style={{ flex: 1, fontSize: 'var(--text-label)' }}
            />
          </div>
          {loading && (
            <div style={{ padding: '10px 12px', fontSize: 'var(--text-label)', color: 'var(--text-faint)' }}>
              {searching ? '搜索中…' : '装配中…'}
            </div>
          )}
          {!loading && items.length === 0 && (
            <div style={{ padding: '10px 12px', fontSize: 'var(--text-label)', color: 'var(--text-faint)' }}>
              {searching ? `没有标题命中「${serverQuery}」的条目` : '暂无可引用条目（基质随写入逐步填充）'}
            </div>
          )}
          {!loading && (kind
            // 单来源筛选：平铺（组只剩一个，分组头是噪音）。
            ? items.map(renderRow)
            : grouped.map(({ group, total, show, expanded }) => (
                <div key={group}>
                  <div
                    onClick={() => !searching && toggleGroup(group)}
                    style={{
                      display: 'flex', alignItems: 'center', gap: 6, padding: '5px 10px',
                      cursor: searching ? 'default' : 'pointer', userSelect: 'none',
                      fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)',
                      letterSpacing: '.14em', textTransform: 'uppercase', color: 'var(--text-3)',
                      borderTop: '1px solid var(--border)',
                    }}
                  >
                    {!searching && (
                      <Icon
                        name="chevron"
                        size={12}
                        style={{ transform: expanded ? 'none' : 'rotate(-90deg)', transition: 'transform .15s' }}
                      />
                    )}
                    <span>{group}</span>
                    <span className="chip" style={{ marginLeft: 'auto' }}>{total}</span>
                  </div>
                  {expanded && show.map(renderRow)}
                  {expanded && !searching && show.length < total && (
                    <div style={{ padding: '2px 12px 6px', fontSize: 'var(--text-caption)', color: 'var(--text-faint)' }}>
                      还有 {total - show.length} 条…输入关键词搜索
                    </div>
                  )}
                </div>
              )))}
          {!searching && !loading && (
            <div
              style={{
                padding: '7px 12px', fontSize: 'var(--text-caption)', color: 'var(--text-faint)',
                borderTop: '1px solid var(--border)', position: 'sticky', bottom: 0,
                background: 'var(--bg-2)',
              }}
            >
              输入关键词可搜索全部 {searchableCount} 类来源的完整历史
            </div>
          )}
        </div>
      )}
    </div>
  );
}
