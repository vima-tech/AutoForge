import { useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import axios from "axios";
import toast from "react-hot-toast";

interface ChangeRequest {
  id: string;
  status: string;
  iteration_count: number | null;
  admin_suggestions_1: string | null;
  admin_suggestions_2: string | null;
}

interface PreviewEnv {
  id: string;
  env_type: string;
  preview_url: string | null;
  status: string;
}

export function Review2Page() {
  const { crId } = useParams<{ crId: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [suggestions, setSuggestions] = useState("");

  const { data: cr } = useQuery<ChangeRequest>({
    queryKey: ["change-requests", crId],
    queryFn: async () => {
      const { data } = await axios.get(`/api/v1/change-requests/${crId}`);
      return data;
    },
    enabled: !!crId,
  });

  const decideMutation = useMutation({
    mutationFn: async (decision: "approved" | "rejected" | "revision") => {
      const { data } = await axios.post(
        `/api/v1/reviews/change-requests/${crId}/review-2`,
        { decision, suggestions: suggestions || null, stage: "review_2" },
        { headers: { "x-admin-id": "admin" } }
      );
      return data;
    },
    onSuccess: (_, decision) => {
      const messages = { approved: "已批准合并", rejected: "已拒绝", revision: "已发送修改请求" };
      toast.success(messages[decision]);
      queryClient.invalidateQueries({ queryKey: ["change-requests"] });
      navigate("/");
    },
    onError: () => toast.error("操作失败"),
  });

  const iterCount = cr?.iteration_count ?? 1;
  const showIterWarning = iterCount >= 3;

  if (!crId) return <p style={{ color: "var(--text-muted)" }}>请从需求队列选择一个待审核变更请求</p>;
  if (!cr) return <p style={{ color: "var(--text-muted)" }}>加载中…</p>;

  return (
    <div>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 20 }}>
        <h1 style={{ fontSize: 18, fontWeight: 700 }}>审核节点 2：实现报告</h1>
        <span style={{ color: "var(--text-muted)", fontSize: 13 }}>第 {iterCount} 次迭代</span>
      </div>

      {showIterWarning && (
        <div style={{
          background: "rgba(251,191,36,0.1)", border: "1px solid rgba(251,191,36,0.3)",
          borderRadius: 8, padding: 16, marginBottom: 20,
        }}>
          <strong style={{ color: "var(--warning)" }}>已迭代 {iterCount} 轮，建议考虑：</strong>
          <ul style={{ marginTop: 8, paddingLeft: 20, color: "var(--text-muted)", fontSize: 13 }}>
            <li>手动介入直接修改代码</li>
            <li>重新描述需求，提供更明确的约束</li>
            <li>拆分为更小的子需求</li>
          </ul>
        </div>
      )}

      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 20 }}>
        {/* Left: Report */}
        <div style={{ background: "var(--surface)", border: "1px solid var(--border)", borderRadius: 8, padding: 20 }}>
          <h3 style={{ fontSize: 13, fontWeight: 600, marginBottom: 12 }}>实现报告</h3>
          <p style={{ color: "var(--text-muted)", fontSize: 13 }}>
            Claude Code 实现报告（M4 里程碑后填充）
          </p>
          {cr.admin_suggestions_1 && (
            <div style={{ marginTop: 16, padding: 12, background: "var(--bg)", borderRadius: 6, fontSize: 12 }}>
              <div style={{ color: "var(--text-muted)", marginBottom: 4 }}>审核节点 1 建议：</div>
              <div>{cr.admin_suggestions_1}</div>
            </div>
          )}
        </div>

        {/* Right: Preview */}
        <div style={{ background: "var(--surface)", border: "1px solid var(--border)", borderRadius: 8, padding: 20 }}>
          <h3 style={{ fontSize: 13, fontWeight: 600, marginBottom: 12 }}>实时预览</h3>
          <div style={{ color: "var(--text-muted)", fontSize: 13 }}>
            预览系统（M5 里程碑后提供 iframe 对比视图）
          </div>
        </div>
      </div>

      <div style={{ marginTop: 20 }}>
        <label style={{ display: "block", fontSize: 13, marginBottom: 8, color: "var(--text-muted)" }}>
          修改建议（选择"修改"时传递给 Claude Code）
        </label>
        <textarea
          value={suggestions}
          onChange={(e) => setSuggestions(e.target.value)}
          rows={4}
          style={{
            width: "100%", background: "var(--surface)", border: "1px solid var(--border)",
            borderRadius: 6, padding: 10, color: "var(--text)", resize: "vertical",
          }}
          placeholder="描述需要修改的地方…"
        />
      </div>

      <div style={{ display: "flex", gap: 12, marginTop: 16 }}>
        <button
          className="btn-primary"
          onClick={() => decideMutation.mutate("approved")}
          disabled={decideMutation.isPending}
        >
          批准合并
        </button>
        <button
          className="btn-ghost"
          onClick={() => decideMutation.mutate("revision")}
          disabled={decideMutation.isPending}
        >
          修改
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
