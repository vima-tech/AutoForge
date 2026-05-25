export function ScoreBar({ score, label }: { score: number | null; label: string }) {
  if (score === null) return null;
  const pct = Math.round(score * 100);
  const color = pct >= 70 ? "var(--green)" : pct >= 40 ? "var(--yellow)" : "var(--red)";

  return (
    <div style={{ minWidth: 140 }}>
      <div className="section-label" style={{ marginBottom: 6 }}>{label}</div>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <div className="score-bar" style={{ flex: 1 }}>
          <div className="score-bar-fill" style={{ width: `${pct}%`, background: color }} />
        </div>
        <span style={{ fontSize: 13, fontWeight: 600, color, minWidth: 32 }}>{pct}%</span>
      </div>
    </div>
  );
}
