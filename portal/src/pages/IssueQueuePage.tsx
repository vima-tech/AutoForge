import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import axios from "axios";

interface Issue {
  id: string;
  title: string;
  source_type: string;
  category: string | null;
  severity: string | null;
  status: string;
  priority: number | null;
  created_at: string;
}

const severityBadge = (s: string | null) => {
  if (!s) return null;
  const cls = { critical: "badge-danger", high: "badge-warning", medium: "badge-info", low: "badge-info" }[s] ?? "badge-info";
  return <span className={`badge ${cls}`}>{s}</span>;
};

const statusLabel: Record<string, string> = {
  pending_analysis: "待分析",
  pending_review: "待审核",
  approved: "已批准",
  rejected: "已拒绝",
  in_progress: "执行中",
  completed: "已完成",
};

export function IssueQueuePage() {
  const { data: issues = [], isLoading } = useQuery<Issue[]>({
    queryKey: ["issues"],
    queryFn: async () => {
      const { data } = await axios.get("/api/v1/issues");
      return data;
    },
    refetchInterval: 15_000,
  });

  return (
    <div>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 20 }}>
        <h1 style={{ fontSize: 20, fontWeight: 700 }}>需求队列</h1>
        <span style={{ color: "var(--text-muted)", fontSize: 13 }}>{issues.length} 条需求</span>
      </div>

      {isLoading ? (
        <p style={{ color: "var(--text-muted)" }}>加载中…</p>
      ) : issues.length === 0 ? (
        <p style={{ color: "var(--text-muted)" }}>暂无需求</p>
      ) : (
        <table style={{ width: "100%", borderCollapse: "collapse" }}>
          <thead>
            <tr style={{ borderBottom: "1px solid var(--border)", color: "var(--text-muted)", fontSize: 12, textAlign: "left" }}>
              <th style={{ padding: "8px 12px" }}>标题</th>
              <th style={{ padding: "8px 12px" }}>来源</th>
              <th style={{ padding: "8px 12px" }}>严重级别</th>
              <th style={{ padding: "8px 12px" }}>状态</th>
              <th style={{ padding: "8px 12px" }}>操作</th>
            </tr>
          </thead>
          <tbody>
            {issues.map((issue) => (
              <tr key={issue.id} style={{ borderBottom: "1px solid var(--border)" }}>
                <td style={{ padding: "10px 12px", maxWidth: 400 }}>{issue.title}</td>
                <td style={{ padding: "10px 12px", color: "var(--text-muted)" }}>{issue.source_type}</td>
                <td style={{ padding: "10px 12px" }}>{severityBadge(issue.severity)}</td>
                <td style={{ padding: "10px 12px", color: "var(--text-muted)" }}>
                  {statusLabel[issue.status] ?? issue.status}
                </td>
                <td style={{ padding: "10px 12px" }}>
                  {issue.status === "pending_review" && (
                    <Link to={`/review-1/${issue.id}`} style={{ fontSize: 12 }}>审核</Link>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
