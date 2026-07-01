import React, { useEffect, useMemo, useRef, useState } from 'react';
import Icon from './Icon';
import Select from './Select';
import { listContextItems, type ContextItem } from '../services';

/**
 * 全量上下文基质 · 内联引用选择器（《全量上下文基质·万物可引》契约 §7）。
 *
 * 一个组件吃掉所有功能点的「引用上下文」：会议室 @、孵化台起草、需求提交、审核建议…
 * 复用 `mention-pop / mention-row` 弹层 + `Select` 来源筛选，底层调 `list_context_items`
 * 活查全量来源（需求/会议/日志/trace/规格/.autoforge 文件…）。**不做角色视角，只按来源筛。**
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
  const [kind, setKind] = useState('');
  const [items, setItems] = useState<ContextItem[]>([]);
  const [loading, setLoading] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  // 打开时拉取（按当前来源筛选）。
  useEffect(() => {
    if (!open || !projectId) return;
    setLoading(true);
    const kinds = kind ? [kind] : (sourceKinds && sourceKinds.length ? sourceKinds : undefined);
    listContextItems(projectId, kinds, 200)
      .then(setItems)
      .catch(() => setItems([]))
      .finally(() => setLoading(false));
  }, [open, projectId, kind, sourceKinds]);

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

  // 可选来源下拉：由实际返回的条目 + 限定集合推导。
  const kindOptions = useMemo(() => {
    const base = sourceKinds && sourceKinds.length
      ? sourceKinds
      : Array.from(new Set(items.map(i => i.source_kind)));
    const opts = base.map(k => ({ value: k, label: kindLabel(k) }));
    return [{ value: '', label: '全部来源' }, ...opts];
  }, [items, sourceKinds]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = q
      ? items.filter(i => (i.title || i.id).toLowerCase().includes(q) || i.source_kind.includes(q))
      : items;
    return list.slice(0, 200);
  }, [items, query]);

  const pick = (it: ContextItem) => { onPick(it); setOpen(false); setQuery(''); };

  const popStyle: React.CSSProperties = {
    left: 0, right: 'auto', width: 340, maxHeight: 380, overflowY: 'auto', zIndex: 80,
    ...(placement === 'up'
      ? { bottom: 'calc(100% + 6px)', top: 'auto', marginBottom: 0 }
      : { top: 'calc(100% + 6px)', bottom: 'auto', marginTop: 0 }),
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
              placeholder="🔍 过滤…"
              autoFocus
              style={{ flex: 1, fontSize: 'var(--text-label)' }}
            />
          </div>
          {loading && <div style={{ padding: '10px 12px', fontSize: 'var(--text-label)', color: 'var(--text-faint)' }}>装配中…</div>}
          {!loading && filtered.length === 0 && (
            <div style={{ padding: '10px 12px', fontSize: 'var(--text-label)', color: 'var(--text-faint)' }}>
              暂无可引用条目（基质随写入逐步填充）
            </div>
          )}
          {filtered.map(it => (
            <div key={it.id} className="mention-row" onClick={() => pick(it)}>
              <span className="chip" style={{ flexShrink: 0 }}>{kindLabel(it.source_kind)}</span>
              <div style={{ minWidth: 0, flex: 1 }}>
                <div className="nm" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {it.title || it.id}
                </div>
              </div>
              {it.trust === 'external_untrusted' && <span className="chip amber" style={{ flexShrink: 0 }}>外部</span>}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
