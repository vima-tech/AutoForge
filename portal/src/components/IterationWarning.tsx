export function IterationWarning({
  count,
  crId,
}: {
  count: number;
  crId: string;
}) {
  if (count < 3) return null;

  return (
    <div
      style={{
        background: "rgba(251,191,36,0.08)",
        border: "1px solid rgba(251,191,36,0.3)",
        borderRadius: 8,
        padding: "14px 16px",
        marginBottom: 16,
      }}
    >
      <div style={{ fontWeight: 600, color: "var(--yellow)", marginBottom: 8 }}>
        ⚠ 已迭代 {count} 轮（软上限：3 轮）
      </div>
      <div style={{ color: "var(--muted)", fontSize: 13, lineHeight: 1.7 }}>
        建议考虑：
        <ul style={{ paddingLeft: 18, marginTop: 4 }}>
          <li>手动介入直接修改代码（worktree: <code>autoforge/cr-{crId.slice(0, 8)}</code>）</li>
          <li>重新描述需求，提供更明确的约束</li>
          <li>拆分为更小的子需求</li>
        </ul>
      </div>
    </div>
  );
}
