import { useSystemStatus } from "../hooks/useSystemStatus";

const stageLabel = { normal: "正常运行", throttled: "单线程降速", paused: "已暂停" } as const;
const stageColor = { normal: "var(--success)", throttled: "var(--warning)", paused: "var(--danger)" } as const;

export function SystemStatusPage() {
  const { data: status, isLoading } = useSystemStatus();

  if (isLoading) return <p style={{ color: "var(--text-muted)" }}>加载中…</p>;
  if (!status) return <p style={{ color: "var(--text-muted)" }}>无法获取系统状态</p>;

  const { concurrency: c } = status;
  const stage = c.stage as keyof typeof stageLabel;

  return (
    <div style={{ maxWidth: 600 }}>
      <h1 style={{ fontSize: 20, fontWeight: 700, marginBottom: 24 }}>系统状态</h1>

      <div style={{
        background: "var(--surface)", border: "1px solid var(--border)",
        borderRadius: 8, padding: 24, marginBottom: 20,
      }}>
        <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 20 }}>
          <div style={{
            width: 12, height: 12, borderRadius: "50%",
            background: stageColor[stage] ?? "var(--text-muted)",
          }} />
          <span style={{ fontSize: 16, fontWeight: 600, color: stageColor[stage] }}>
            {stageLabel[stage] ?? stage}
          </span>
        </div>

        {[
          ["活跃槽位", `${c.active_slots} / ${c.max_slots}`],
          ["待审核积压", `${c.pending_review_count} / ${c.pause_threshold} (暂停阈值)`],
          ["当前有效并发数", String(c.effective_concurrency)],
        ].map(([label, value]) => (
          <div key={label} style={{
            display: "flex", justifyContent: "space-between",
            padding: "10px 0", borderBottom: "1px solid var(--border)", fontSize: 14,
          }}>
            <span style={{ color: "var(--text-muted)" }}>{label}</span>
            <span style={{ fontWeight: 500 }}>{value}</span>
          </div>
        ))}
      </div>

      {stage === "paused" && (
        <div style={{
          background: "rgba(248,113,113,0.1)", border: "1px solid rgba(248,113,113,0.3)",
          borderRadius: 8, padding: 16,
        }}>
          <strong style={{ color: "var(--danger)" }}>系统已暂停</strong>
          <p style={{ marginTop: 8, color: "var(--text-muted)", fontSize: 13 }}>
            待审核积压已达上限（{c.pending_review_count} 条）。请完成审核后系统将自动恢复。
            外部输入已进入暂存队列，不会丢失。
          </p>
        </div>
      )}
    </div>
  );
}
