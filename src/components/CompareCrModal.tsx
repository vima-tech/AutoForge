import React, { useState, useEffect } from 'react';
import Icon from './Icon';
import Select from './Select';
import { getCodeDiff } from '../services';

interface Props {
  currentCrId: string;
  currentLabel: string;
  /** 同项目其它 CR 候选（不含当前）。 */
  candidates: { value: string; label: string }[];
  onClose: () => void;
}

/** 一栏 CR diff（标题 + 等宽 diff，带极简增删着色）。 */
function DiffColumn({ title, diff, loading }: { title: string; diff: string; loading: boolean }) {
  return (
    <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', border: '1px solid var(--border)', borderRadius: 'var(--radius)', overflow: 'hidden' }}>
      <div style={{ padding: '8px 12px', borderBottom: '1px solid var(--border)', background: 'var(--bg-2)', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-caption)', color: 'var(--text-2)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
        {title}
      </div>
      <div className="scroll" style={{ flex: 1, background: 'var(--code-bg)', overflow: 'auto' }}>
        {loading ? (
          <div style={{ padding: 16, color: 'var(--text-3)', fontSize: 'var(--text-control)' }}>加载中…</div>
        ) : diff.trim() ? (
          <pre style={{ margin: 0, padding: '10px 12px', fontFamily: 'var(--font-mono)', fontSize: 'var(--text-label)', lineHeight: 'var(--leading-relaxed)' }}>
            {diff.split('\n').map((ln, i) => (
              <div key={i} style={{
                color: ln.startsWith('+') && !ln.startsWith('+++') ? 'var(--green)'
                  : ln.startsWith('-') && !ln.startsWith('---') ? 'var(--red)'
                  : ln.startsWith('@@') ? 'var(--blue)' : 'var(--text-2)',
                whiteSpace: 'pre-wrap', wordBreak: 'break-all',
              }}>{ln || ' '}</div>
            ))}
          </pre>
        ) : (
          <div style={{ padding: 16, color: 'var(--text-faint)', fontSize: 'var(--text-control)' }}>无 diff（worktree 已清理或空改动）</div>
        )}
      </div>
    </div>
  );
}

/**
 * CR 对比模式：把当前 CR 与同项目另一 CR 的代码 diff 并排展示，
 * 便于审核者横向比较两次变更（如「这次改动 vs 上次类似改动」）。
 * 遮罩不点击关闭（DESIGN 约定），仅 ✕ / Esc 关闭。
 */
export default function CompareCrModal({ currentCrId, currentLabel, candidates, onClose }: Props) {
  const [otherId, setOtherId] = useState('');
  const [diffA, setDiffA] = useState('');
  const [diffB, setDiffB] = useState('');
  const [loadingA, setLoadingA] = useState(true);
  const [loadingB, setLoadingB] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  useEffect(() => {
    let alive = true;
    setLoadingA(true);
    getCodeDiff(currentCrId).then(d => { if (alive) setDiffA(d); }).catch(() => { if (alive) setDiffA(''); }).finally(() => { if (alive) setLoadingA(false); });
    return () => { alive = false; };
  }, [currentCrId]);

  useEffect(() => {
    if (!otherId) { setDiffB(''); return; }
    let alive = true;
    setLoadingB(true);
    getCodeDiff(otherId).then(d => { if (alive) setDiffB(d); }).catch(() => { if (alive) setDiffB(''); }).finally(() => { if (alive) setLoadingB(false); });
    return () => { alive = false; };
  }, [otherId]);

  const otherLabel = candidates.find(c => c.value === otherId)?.label ?? '选择对比的 CR';

  return (
    <div className="modal-mask" style={{ position: 'fixed', inset: 'var(--win-gutter, 0)', borderRadius: 14, background: 'color-mix(in srgb, var(--bg) 72%, transparent)', display: 'grid', placeItems: 'center', zIndex: 60 }}>
      <div className="panel" style={{ width: 'min(1100px, 92vw)', height: 'min(80vh, 760px)', display: 'flex', flexDirection: 'column', boxShadow: 'var(--shadow-lg)' }}>
        <div className="panel-head" style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '12px 16px' }}>
          <Icon name="columns" size={16} style={{ color: 'var(--ember)' }} />
          <span style={{ fontWeight: 700, fontSize: 'var(--text-title)' }}>CR 对比</span>
          <Select
            value={otherId}
            onChange={setOtherId}
            options={candidates}
            placeholder="选择对比的 CR…"
            style={{ marginLeft: 8, minWidth: 260 }}
          />
          <button className="icon-btn" title="关闭（Esc）" onClick={onClose} style={{ marginLeft: 'auto' }}>
            <Icon name="x" size={16} />
          </button>
        </div>
        <div style={{ flex: 1, minHeight: 0, display: 'flex', gap: 12, padding: 14 }}>
          <DiffColumn title={`当前 · ${currentLabel}`} diff={diffA} loading={loadingA} />
          <DiffColumn title={otherId ? `对比 · ${otherLabel}` : '选择右侧 CR 进行对比'} diff={diffB} loading={loadingB} />
        </div>
      </div>
    </div>
  );
}
