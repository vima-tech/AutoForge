const SEVERITY_MAP: Record<string, string> = {
  critical: "badge-danger",
  high: "badge-warning",
  medium: "badge-info",
  low: "badge-muted",
};

const STATUS_MAP: Record<string, [string, string]> = {
  pending_analysis: ["badge-muted", "待分析"],
  pending_review: ["badge-warning", "待审核"],
  approved: ["badge-info", "已批准"],
  executing: ["badge-purple", "执行中"],
  pending_review_2: ["badge-warning", "待合并"],
  merged: ["badge-success", "已合并"],
  rejected: ["badge-muted", "已拒绝"],
  failed: ["badge-danger", "失败"],
  in_progress: ["badge-purple", "进行中"],
  completed: ["badge-success", "已完成"],
};

export function SeverityBadge({ severity }: { severity: string | null }) {
  if (!severity) return null;
  return <span className={`badge ${SEVERITY_MAP[severity] ?? "badge-info"}`}>{severity}</span>;
}

export function StatusBadge({ status }: { status: string }) {
  const [cls, label] = STATUS_MAP[status] ?? ["badge-muted", status];
  return <span className={`badge ${cls}`}>{label}</span>;
}

export function CategoryBadge({ category }: { category: string | null }) {
  if (!category) return null;
  return <span className="badge badge-info">{category}</span>;
}
