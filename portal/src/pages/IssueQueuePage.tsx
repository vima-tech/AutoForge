import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import axios from "axios";
import toast from "react-hot-toast";
import { SeverityBadge, StatusBadge, CategoryBadge } from "../components/StatusBadge";
import { getAdminHeaders } from "../store/auth";

interface Issue {
  id: string;
  project_id: string;
  title: string;
  source_type: string;
  category: string | null;
  severity: string | null;
  status: string;
  priority: number | null;
  created_at: string;
}

interface Project {
  id: string;
  name: string;
  slug: string;
}

const ACTION_STATUSES = new Set(["pending_review", "pending_review_2"]);

const SOURCE_ICONS: Record<string, string> = {
  widget: "💬",
  github: "🐙",
  admin: "👤",
  monitor: "📊",
  scan: "🔍",
};

function timeSince(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  if (diff < 60_000) return "刚刚";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`;
  return `${Math.floor(diff / 86_400_000)} 天前`;
}

function SubmitModal({ projects, onClose }: { projects: Project[]; onClose: () => void }) {
  const queryClient = useQueryClient();
  const [form, setForm] = useState({
    project_id: projects[0]?.id || "",
    title: "",
    description: "",
    category: "Bug",
    severity: "medium",
  });

  const mutation = useMutation({
    mutationFn: async () => axios.post("/api/v1/issues", { ...form, source_type: "admin" }, {
      headers: getAdminHeaders(),
    }),
    onSuccess: () => {
      toast.success("需求已提交，分析中…");
      queryClient.invalidateQueries({ queryKey: ["issues"] });
      onClose();
    },
    onError: (e: any) => toast.error(e.response?.data?.detail || "提交失败"),
  });

  return (
    <div style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.6)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 9999 }}>
      <div className="card" style={{ padding: 24, width: 460 }}>
        <h3 style={{ marginBottom: 16, fontWeight: 600 }}>手动提交需求</h3>

        <label style={{ fontSize: 12, color: "var(--muted)" }}>项目</label>
        <select value={form.project_id} onChange={e => setForm({ ...form, project_id: e.target.value })} style={{ marginBottom: 10 }}>
          {projects.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>

        <label style={{ fontSize: 12, color: "var(--muted)" }}>标题 *</label>
        <input
          value={form.title}
          onChange={e => setForm({ ...form, title: e.target.value })}
          placeholder="用一句话描述问题或建议…"
          style={{ marginBottom: 10 }}
        />

        <label style={{ fontSize: 12, color: "var(--muted)" }}>详细说明（可选）</label>
        <textarea
          value={form.description}
          onChange={e => setForm({ ...form, description: e.target.value })}
          rows={3}
          placeholder="复现步骤、期望效果…"
          style={{ marginBottom: 10 }}
        />

        <div style={{ display: "flex", gap: 12, marginBottom: 16 }}>
          <div style={{ flex: 1 }}>
            <label style={{ fontSize: 12, color: "var(--muted)" }}>类别</label>
            <select value={form.category} onChange={e => setForm({ ...form, category: e.target.value })}>
              {["Bug", "Feature", "Improvement", "Security", "Debt"].map(c => <option key={c}>{c}</option>)}
            </select>
          </div>
          <div style={{ flex: 1 }}>
            <label style={{ fontSize: 12, color: "var(--muted)" }}>严重级别</label>
            <select value={form.severity} onChange={e => setForm({ ...form, severity: e.target.value })}>
              {["critical", "high", "medium", "low"].map(s => <option key={s}>{s}</option>)}
            </select>
          </div>
        </div>

        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button className="btn btn-ghost" onClick={onClose}>取消</button>
          <button
            className="btn btn-primary"
            onClick={() => mutation.mutate()}
            disabled={!form.title.trim() || !form.project_id || mutation.isPending}
          >
            {mutation.isPending ? "提交中…" : "提交需求"}
          </button>
        </div>
      </div>
    </div>
  );
}

export function IssueQueuePage() {
  const [showSubmit, setShowSubmit] = useState(false);
  const [filter, setFilter] = useState<string>("all");

  const { data: issues = [], isLoading } = useQuery<Issue[]>({
    queryKey: ["issues", filter],
    queryFn: async () => {
      const params = filter !== "all" ? `?status=${filter}` : "";
      return (await axios.get(`/api/v1/issues${params}`)).data;
    },
    refetchInterval: 12_000,
  });

  const { data: projects = [] } = useQuery<Project[]>({
    queryKey: ["projects"],
    queryFn: async () => (await axios.get("/api/v1/projects")).data,
  });

  const actionItems = issues.filter(i => ACTION_STATUSES.has(i.status));
  const executingItems = issues.filter(i => i.status === "executing" || i.status === "approved");
  const pendingItems = issues.filter(i => i.status === "pending_analysis");
  const completedItems = issues.filter(i => ["completed", "merged", "rejected"].includes(i.status)).slice(0, 10);

  const renderRow = (issue: Issue) => (
    <tr key={issue.id} style={{ borderBottom: "1px solid var(--border)", transition: "background 0.1s" }}>
      <td style={{ padding: "12px 16px", maxWidth: 380 }}>
        <div style={{ fontWeight: 500, marginBottom: 3 }}>{issue.title}</div>
        <div style={{ fontSize: 12, color: "var(--muted)" }}>
          {SOURCE_ICONS[issue.source_type] || "•"} {issue.source_type} · {timeSince(issue.created_at)}
          {issue.priority && ` · P${issue.priority}`}
        </div>
      </td>
      <td style={{ padding: "12px 8px" }}>
        <SeverityBadge severity={issue.severity} />
      </td>
      <td style={{ padding: "12px 8px" }}>
        <CategoryBadge category={issue.category} />
      </td>
      <td style={{ padding: "12px 8px" }}>
        <StatusBadge status={issue.status} />
      </td>
      <td style={{ padding: "12px 16px" }}>
        {issue.status === "pending_review" && (
          <Link to={`/review-1/${issue.id}`} className="btn btn-primary btn-sm">审核 →</Link>
        )}
        {issue.status === "pending_review_2" && (
          <Link to={`/review-2/${issue.id}`} className="btn btn-success btn-sm">查看实现 →</Link>
        )}
        {(issue.status === "executing" || issue.status === "approved") && (
          <span style={{ fontSize: 12, color: "var(--purple)", display: "flex", alignItems: "center", gap: 6 }}>
            <div className="spinner" /> 运行中
          </span>
        )}
      </td>
    </tr>
  );

  const Section = ({ title, items, emptyText }: { title: string; items: Issue[]; emptyText?: string }) => (
    items.length > 0 ? (
      <>
        <tr>
          <td colSpan={5} style={{ padding: "16px 16px 8px", fontSize: 11, fontWeight: 600, textTransform: "uppercase", letterSpacing: "0.05em", color: "var(--muted)" }}>
            {title} ({items.length})
          </td>
        </tr>
        {items.map(renderRow)}
      </>
    ) : null
  );

  return (
    <div>
      {showSubmit && <SubmitModal projects={projects} onClose={() => setShowSubmit(false)} />}

      <div className="page-header">
        <h1>需求队列</h1>
        <div style={{ display: "flex", gap: 8 }}>
          <select value={filter} onChange={e => setFilter(e.target.value)} style={{ width: 130 }}>
            <option value="all">全部状态</option>
            <option value="pending_review">待审核 1</option>
            <option value="pending_review_2">待合并</option>
            <option value="executing">执行中</option>
            <option value="pending_analysis">待分析</option>
          </select>
          <button className="btn btn-primary btn-sm" onClick={() => setShowSubmit(true)}>+ 手动提交</button>
        </div>
      </div>

      {isLoading ? (
        <div className="empty-state"><div className="spinner" /></div>
      ) : issues.length === 0 ? (
        <div className="empty-state">
          <div className="icon">✨</div>
          <p>暂无需求，等待反馈中</p>
        </div>
      ) : (
        <div className="card" style={{ overflow: "hidden" }}>
          <table style={{ width: "100%", borderCollapse: "collapse" }}>
            <thead>
              <tr style={{ fontSize: 11, color: "var(--muted)", textAlign: "left", borderBottom: "1px solid var(--border)" }}>
                <th style={{ padding: "10px 16px", fontWeight: 600, textTransform: "uppercase" }}>标题</th>
                <th style={{ padding: "10px 8px", fontWeight: 600, textTransform: "uppercase" }}>级别</th>
                <th style={{ padding: "10px 8px", fontWeight: 600, textTransform: "uppercase" }}>类别</th>
                <th style={{ padding: "10px 8px", fontWeight: 600, textTransform: "uppercase" }}>状态</th>
                <th style={{ padding: "10px 16px", fontWeight: 600, textTransform: "uppercase" }}>操作</th>
              </tr>
            </thead>
            <tbody>
              <Section title="需要行动" items={actionItems} />
              <Section title="执行中" items={executingItems} />
              <Section title="待分析" items={pendingItems} />
              <Section title="已完成（最近）" items={completedItems} />
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
