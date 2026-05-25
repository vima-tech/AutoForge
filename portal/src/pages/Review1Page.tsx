import { useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import axios from "axios";
import toast from "react-hot-toast";
import { SeverityBadge, CategoryBadge } from "../components/StatusBadge";
import { ScoreBar } from "../components/ScoreBar";
import { getAdminHeaders } from "../store/auth";

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
  authenticity_score: number;
  feasibility_score: number | null;
  priority_suggestion: number | null;
  category_suggestion: string | null;
  severity_suggestion: string | null;
  affected_modules: { modules?: string[] } | null;
  analysis_summary: string | null;
  raw_llm_output: Record<string, unknown> | null;
}

function ConfirmDialog({
  onConfirm,
  onCancel,
  suggestions,
  onSuggestions,
}: {
  onConfirm: (reason: string) => void;
  onCancel: () => void;
  suggestions: string;
  onSuggestions: (v: string) => void;
}) {
  return (
    <div style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.6)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 9999 }}>
      <div className="card" style={{ padding: 24, width: 380 }}>
        <h3 style={{ marginBottom: 12, fontWeight: 600 }}>确认拒绝？</h3>
        <p style={{ color: "var(--muted)", fontSize: 13, marginBottom: 12 }}>
          拒绝后此需求将不再进入执行队列。
        </p>
        <label style={{ fontSize: 12, color: "var(--muted)" }}>拒绝原因（可选）</label>
        <textarea
          value={suggestions}
          onChange={e => onSuggestions(e.target.value)}
          rows={2}
          style={{ marginTop: 4, marginBottom: 16 }}
          placeholder="与 #123 重复、超出范围…"
        />
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button className="btn btn-ghost" onClick={onCancel}>取消</button>
          <button className="btn btn-danger" onClick={() => onConfirm(suggestions)}>确认拒绝</button>
        </div>
      </div>
    </div>
  );
}

export function Review1Page() {
  const { issueId } = useParams<{ issueId: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [suggestions, setSuggestions] = useState("");
  const [showRejectConfirm, setShowRejectConfirm] = useState(false);
  const [expandReport, setExpandReport] = useState(false);

  const { data: issue, isLoading: issueLoading } = useQuery<Issue>({
    queryKey: ["issue", issueId],
    queryFn: async () => (await axios.get(`/api/v1/issues/${issueId}`)).data,
    enabled: !!issueId,
  });

  const { data: analysis } = useQuery<Analysis>({
    queryKey: ["analysis", issueId],
    queryFn: async () => (await axios.get(`/api/v1/issues/${issueId}/analysis`)).data,
    enabled: !!issueId && !!issue,
    retry: false,
  });

  const decideMutation = useMutation({
    mutationFn: async ({ decision, reason }: { decision: string; reason: string }) => {
      return (await axios.post(
        `/api/v1/reviews/issues/${issueId}/review-1`,
        { decision, suggestions: reason || null, stage: "review_1" },
        { headers: getAdminHeaders() }
      )).data;
    },
    onSuccess: (_, { decision }) => {
      toast.success(decision === "approved" ? "已批准，Claude Code 将开始执行" : "已拒绝");
      queryClient.invalidateQueries({ queryKey: ["issues"] });
      navigate("/");
    },
    onError: (e: any) => toast.error(e.response?.data?.detail || "操作失败"),
  });

  if (!issueId) return (
    <div className="empty-state">
      <div className="icon">📋</div>
      <p>从需求队列选择一条待审核需求</p>
    </div>
  );
  if (issueLoading) return <div className="empty-state"><div className="spinner" /></div>;
  if (!issue) return <div className="empty-state">需求不存在</div>;

  const modules = analysis?.affected_modules?.modules || [];

  return (
    <div style={{ maxWidth: 800 }}>
      {showRejectConfirm && (
        <ConfirmDialog
          suggestions={suggestions}
          onSuggestions={setSuggestions}
          onCancel={() => setShowRejectConfirm(false)}
          onConfirm={(reason) => {
            setShowRejectConfirm(false);
            decideMutation.mutate({ decision: "rejected", reason });
          }}
        />
      )}

      <div className="page-header">
        <h1>审核节点 1：需求分析报告</h1>
        <button className="btn btn-ghost btn-sm" onClick={() => navigate("/")}>← 返回</button>
      </div>

      {/* Issue card */}
      <div className="card" style={{ padding: 20, marginBottom: 16 }}>
        <div style={{ display: "flex", alignItems: "flex-start", justifyContent: "space-between", gap: 12, marginBottom: 10 }}>
          <h2 style={{ fontSize: 15, fontWeight: 600, flex: 1 }}>{issue.title}</h2>
          <div style={{ display: "flex", gap: 6, flexShrink: 0 }}>
            <SeverityBadge severity={issue.severity} />
            <CategoryBadge category={issue.category} />
            <span className="badge badge-muted">{issue.source_type}</span>
          </div>
        </div>
        {issue.description && (
          <p style={{ color: "var(--muted)", fontSize: 13, lineHeight: 1.6 }}>{issue.description}</p>
        )}
      </div>

      {/* Analysis report */}
      <div className="card" style={{ padding: 20, marginBottom: 16 }}>
        <div className="section-label" style={{ marginBottom: 16 }}>Agent 分析报告</div>

        {!analysis ? (
          <div style={{ color: "var(--muted)", fontSize: 13, display: "flex", alignItems: "center", gap: 8 }}>
            <div className="spinner" />
            分析中，请稍候…
          </div>
        ) : (
          <>
            <div style={{ display: "flex", gap: 20, flexWrap: "wrap", marginBottom: 16 }}>
              <ScoreBar score={analysis.authenticity_score} label="真实性" />
              <ScoreBar score={analysis.feasibility_score} label="可行性" />
              {analysis.priority_suggestion && (
                <div>
                  <div className="section-label">优先级建议</div>
                  <div style={{ fontSize: 20, fontWeight: 700, color: "var(--blue)" }}>
                    P{analysis.priority_suggestion}
                  </div>
                </div>
              )}
            </div>

            {analysis.analysis_summary && (
              <div style={{ marginBottom: 12 }}>
                <div className="section-label">分析摘要</div>
                <p style={{ fontSize: 13, lineHeight: 1.7, marginTop: 6, color: "var(--text)" }}>
                  {analysis.analysis_summary}
                </p>
              </div>
            )}

            {modules.length > 0 && (
              <div style={{ marginBottom: 8 }}>
                <div className="section-label">影响模块</div>
                <div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginTop: 4 }}>
                  {modules.map(m => <span key={m} className="badge badge-info">{m}</span>)}
                </div>
              </div>
            )}

            <button
              className="btn btn-ghost btn-sm"
              style={{ marginTop: 8 }}
              onClick={() => setExpandReport(!expandReport)}
            >
              {expandReport ? "▴ 收起详情" : "▾ 展开完整报告"}
            </button>

            {expandReport && analysis.raw_llm_output && (
              <div className="log-block" style={{ marginTop: 12, maxHeight: 300 }}>
                {JSON.stringify(analysis.raw_llm_output, null, 2)}
              </div>
            )}
          </>
        )}
      </div>

      {/* Suggestions */}
      <div style={{ marginBottom: 16 }}>
        <label style={{ display: "block", fontSize: 12, color: "var(--muted)", marginBottom: 6 }}>
          附加建议（可选，传递给 Claude Code）
        </label>
        <textarea
          value={suggestions}
          onChange={e => setSuggestions(e.target.value)}
          rows={3}
          placeholder="具体实现约束、参考文件路径、特别注意事项…"
        />
      </div>

      {/* Decision bar */}
      <div
        style={{
          display: "flex",
          gap: 10,
          padding: "14px 0",
          borderTop: "1px solid var(--border)",
        }}
      >
        <button
          className="btn btn-success"
          onClick={() => decideMutation.mutate({ decision: "approved", reason: suggestions })}
          disabled={decideMutation.isPending}
        >
          {decideMutation.isPending ? <><div className="spinner" /> 处理中…</> : "✓ 批准，进入执行队列"}
        </button>
        <button
          className="btn btn-danger"
          onClick={() => setShowRejectConfirm(true)}
          disabled={decideMutation.isPending}
        >
          ✕ 拒绝
        </button>
        <button className="btn btn-ghost" onClick={() => navigate("/")}>稍后决策</button>
      </div>

      <div style={{ fontSize: 11, color: "var(--disabled)", marginTop: 8 }}>
        快捷键：<kbd style={{ background: "var(--elevated)", padding: "1px 6px", borderRadius: 4 }}>A</kbd> 批准 &nbsp;
        <kbd style={{ background: "var(--elevated)", padding: "1px 6px", borderRadius: 4 }}>Esc</kbd> 返回
      </div>
    </div>
  );
}
