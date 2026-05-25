import { useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import axios from "axios";
import toast from "react-hot-toast";
import { IterationWarning } from "../components/IterationWarning";
import { PreviewFrame } from "../components/PreviewFrame";

interface ChangeRequest {
  id: string;
  status: string;
  issue_id: string;
  admin_suggestions_1: string | null;
  admin_suggestions_2: string | null;
}

interface WorktreeSession {
  id: string;
  iteration_count: number;
  report_content: string | null;
  status: string;
}

interface PreviewEnv {
  id: string;
  env_type: string;
  preview_url: string | null;
  status: string;
}

function ConfirmMergeDialog({ onConfirm, onCancel }: { onConfirm: () => void; onCancel: () => void }) {
  return (
    <div style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.6)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 9999 }}>
      <div className="card" style={{ padding: 24, width: 400 }}>
        <h3 style={{ marginBottom: 12, fontWeight: 600 }}>确认合并到 dev 分支？</h3>
        <ul style={{ color: "var(--muted)", fontSize: 13, lineHeight: 2, paddingLeft: 18, marginBottom: 20 }}>
          <li>worktree 分支将合并到 dev</li>
          <li>预览环境将销毁，并发槽位释放</li>
          <li>测试 Agent 将在合并后自动执行</li>
        </ul>
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button className="btn btn-ghost" onClick={onCancel}>取消</button>
          <button className="btn btn-success" onClick={onConfirm}>✓ 确认合并</button>
        </div>
      </div>
    </div>
  );
}

export function Review2Page() {
  const { crId } = useParams<{ crId: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [suggestions, setSuggestions] = useState("");
  const [showMergeConfirm, setShowMergeConfirm] = useState(false);

  const { data: cr } = useQuery<ChangeRequest>({
    queryKey: ["cr", crId],
    queryFn: async () => (await axios.get(`/api/v1/change-requests/${crId}`)).data,
    enabled: !!crId,
    refetchInterval: 10_000,
  });

  // Latest worktree session
  const { data: sessions = [] } = useQuery<WorktreeSession[]>({
    queryKey: ["sessions", crId],
    queryFn: async () => (await axios.get(`/api/v1/worktree-sessions?cr_id=${crId}`)).data,
    enabled: !!crId,
  });
  const latestSession = sessions[0];

  // Preview environments
  const { data: previews = [] } = useQuery<PreviewEnv[]>({
    queryKey: ["previews", cr?.issue_id],
    queryFn: async () => (await axios.get(`/api/v1/preview/${cr?.issue_id}`)).data,
    enabled: !!cr,
    refetchInterval: 5_000,
  });

  const worktreePreview = previews.find(p => p.env_type === "worktree");
  const mainPreview = previews.find(p => p.env_type === "main");

  const decideMutation = useMutation({
    mutationFn: async (decision: string) => {
      return (await axios.post(
        `/api/v1/reviews/change-requests/${crId}/review-2`,
        { decision, suggestions: suggestions || null, stage: "review_2" },
        { headers: { "x-admin-id": "admin" } }
      )).data;
    },
    onSuccess: (_, decision) => {
      const msgs: Record<string, string> = { approved: "已批准合并", rejected: "已拒绝", revision: "修改请求已发送" };
      toast.success(msgs[decision] || "操作成功");
      queryClient.invalidateQueries({ queryKey: ["cr"] });
      navigate("/");
    },
    onError: (e: any) => toast.error(e.response?.data?.detail || "操作失败"),
  });

  const restartMutation = useMutation({
    mutationFn: async (envId: string) => axios.post(`/api/v1/preview/${envId}/restart`),
    onSuccess: () => { toast.success("重启已排队"); queryClient.invalidateQueries({ queryKey: ["previews"] }); },
    onError: () => toast.error("重启失败"),
  });

  if (!crId) return (
    <div className="empty-state"><div className="icon">🔍</div><p>请从队列选择一个待审核变更请求</p></div>
  );
  if (!cr) return <div className="empty-state"><div className="spinner" /></div>;

  const iterCount = latestSession?.iteration_count ?? 1;
  const reportLines = (latestSession?.report_content || "").split("\n");

  return (
    <div style={{ height: "calc(100vh - 48px)", display: "flex", flexDirection: "column" }}>
      {showMergeConfirm && (
        <ConfirmMergeDialog
          onCancel={() => setShowMergeConfirm(false)}
          onConfirm={() => { setShowMergeConfirm(false); decideMutation.mutate("approved"); }}
        />
      )}

      {/* Top bar */}
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "12px 0", borderBottom: "1px solid var(--border)", marginBottom: 16, flexShrink: 0 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <button className="btn btn-ghost btn-sm" onClick={() => navigate("/")}>←</button>
          <span style={{ fontWeight: 600, fontSize: 15 }}>审核节点 2</span>
          <span style={{ color: "var(--muted)", fontSize: 13 }}>第 {iterCount} 次迭代</span>
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn btn-success" onClick={() => setShowMergeConfirm(true)} disabled={decideMutation.isPending}>
            ✓ 批准合并
          </button>
          <button className="btn btn-ghost" onClick={() => decideMutation.mutate("revision")} disabled={decideMutation.isPending}>
            ↺ 修改
          </button>
          <button className="btn btn-danger" onClick={() => decideMutation.mutate("rejected")} disabled={decideMutation.isPending}>
            ✕ 拒绝
          </button>
        </div>
      </div>

      <IterationWarning count={iterCount} crId={crId} />

      {/* Split pane */}
      <div style={{ flex: 1, display: "grid", gridTemplateColumns: "1fr 1fr", gap: 16, overflow: "hidden", minHeight: 0 }}>
        {/* Left: Report */}
        <div style={{ overflow: "auto", display: "flex", flexDirection: "column", gap: 16 }}>
          <div className="card" style={{ padding: 16 }}>
            <div className="section-label" style={{ marginBottom: 10 }}>实现报告</div>
            {latestSession?.report_content ? (
              <div className="log-block" style={{ maxHeight: 400, whiteSpace: "pre-wrap" }}>
                {latestSession.report_content}
              </div>
            ) : (
              <div style={{ color: "var(--muted)", fontSize: 13 }}>
                {cr.status === "executing" ? (
                  <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                    <div className="spinner" /> Claude Code 执行中…
                  </div>
                ) : "报告生成中"}
              </div>
            )}
          </div>

          {cr.admin_suggestions_1 && (
            <div className="card" style={{ padding: 12 }}>
              <div className="section-label">Review 1 附加建议</div>
              <p style={{ fontSize: 13, marginTop: 6, color: "var(--muted)" }}>{cr.admin_suggestions_1}</p>
            </div>
          )}

          <div>
            <label style={{ display: "block", fontSize: 12, color: "var(--muted)", marginBottom: 6 }}>
              修改建议（选择"修改"时传递给 Claude Code）
            </label>
            <textarea
              value={suggestions}
              onChange={e => setSuggestions(e.target.value)}
              rows={4}
              placeholder="描述需要修改的细节…"
            />
          </div>
        </div>

        {/* Right: Preview */}
        <div style={{ overflow: "auto", display: "flex", flexDirection: "column", gap: 12 }}>
          <PreviewFrame
            url={mainPreview?.preview_url ?? null}
            label="🔴 生产版本 (main)"
            indicator="red"
            status={mainPreview?.status ?? "pending"}
          />
          <PreviewFrame
            url={worktreePreview?.preview_url ?? null}
            label="🟢 本次改动"
            indicator="green"
            status={worktreePreview?.status ?? "pending"}
            onRestart={worktreePreview ? () => restartMutation.mutate(worktreePreview.id) : undefined}
          />
        </div>
      </div>
    </div>
  );
}
