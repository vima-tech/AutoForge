import { useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import axios from "axios";
import toast from "react-hot-toast";

interface Issue {
  id: string;
  title: string;
  description: string | null;
  source_type: string;
  severity: string | null;
  category: string | null;
  status: string;
}

interface Analysis {
  id: string;
  authenticity_score: number;
  feasibility_score: number | null;
  priority_suggestion: number | null;
  severity_suggestion: string | null;
  analysis_summary: string | null;
  affected_modules: Record<string, unknown> | null;
}

export function Review1Page() {
  const { issueId } = useParams<{ issueId: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [suggestions, setSuggestions] = useState("");

  const { data: issue } = useQuery<Issue>({
    queryKey: ["issue", issueId],
    queryFn: async () => {
      const { data } = await axios.get(`/api/v1/issues/${issueId}`);
      return data;
    },
    enabled: !!issueId,
  });

  const decideMutation = useMutation({
    mutationFn: async (decision: "approved" | "rejected") => {
      const { data } = await axios.post(
        `/api/v1/reviews/issues/${issueId}/review-1`,
        { decision, suggestions: suggestions || null, stage: "review_1" },
        { headers: { "x-admin-id": "admin" } }
      );
      return data;
    },
    onSuccess: (_, decision) => {
      toast.success(decision === "approved" ? "已批准，进入执行队列" : "已拒绝");
      queryClient.invalidateQueries({ queryKey: ["issues"] });
      navigate("/");
    },
    onError: () => toast.error("操作失败"),
  });

  if (!issueId) return <p style={{ color: "var(--text-muted)" }}>请从需求队列选择一条待审核需求</p>;
  if (!issue) return <p style={{ color: "var(--text-muted)" }}>加载中…</p>;

  return (
    <div style={{ maxWidth: 760 }}>
      <h1 style={{ fontSize: 18, fontWeight: 700, marginBottom: 20 }}>审核节点 1：需求分析报告</h1>

      <section style={{ background: "var(--surface)", border: "1px solid var(--border)", borderRadius: 8, padding: 20, marginBottom: 20 }}>
        <h2 style={{ fontSize: 14, fontWeight: 600, marginBottom: 12 }}>{issue.title}</h2>
        {issue.description && <p style={{ color: "var(--text-muted)", lineHeight: 1.6 }}>{issue.description}</p>}
        <div style={{ marginTop: 12, display: "flex", gap: 8 }}>
          <span className="badge badge-info">{issue.source_type}</span>
          {issue.severity && <span className="badge badge-warning">{issue.severity}</span>}
          {issue.category && <span className="badge badge-info">{issue.category}</span>}
        </div>
      </section>

      <section style={{ background: "var(--surface)", border: "1px solid var(--border)", borderRadius: 8, padding: 20, marginBottom: 20 }}>
        <h3 style={{ fontSize: 13, fontWeight: 600, marginBottom: 12, color: "var(--text-muted)" }}>Agent 分析报告（M3 实现后填充）</h3>
        <p style={{ color: "var(--text-muted)", fontSize: 13 }}>分析 Agent 尚未实现（M3 里程碑）</p>
      </section>

      <section style={{ marginBottom: 20 }}>
        <label style={{ display: "block", fontSize: 13, marginBottom: 8, color: "var(--text-muted)" }}>
          附加建议（可选，传递给 Claude Code）
        </label>
        <textarea
          value={suggestions}
          onChange={(e) => setSuggestions(e.target.value)}
          rows={4}
          style={{
            width: "100%", background: "var(--surface)", border: "1px solid var(--border)",
            borderRadius: 6, padding: 10, color: "var(--text)", resize: "vertical",
          }}
          placeholder="输入对 Claude Code 的实现建议或约束条件…"
        />
      </section>

      <div style={{ display: "flex", gap: 12 }}>
        <button
          className="btn-primary"
          onClick={() => decideMutation.mutate("approved")}
          disabled={decideMutation.isPending}
        >
          批准
        </button>
        <button
          className="btn-danger"
          onClick={() => decideMutation.mutate("rejected")}
          disabled={decideMutation.isPending}
        >
          拒绝
        </button>
        <button className="btn-ghost" onClick={() => navigate("/")}>返回</button>
      </div>
    </div>
  );
}
