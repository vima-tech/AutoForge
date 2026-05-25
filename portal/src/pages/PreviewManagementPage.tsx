import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import axios from "axios";
import toast from "react-hot-toast";

interface PreviewEnv {
  id: string;
  project_id: string;
  env_type: string;
  preview_url: string | null;
  status: string;
  created_at: string;
  ready_at: string | null;
}

const statusBadge = (s: string) => {
  const map: Record<string, string> = {
    ready: "badge-success",
    building: "badge-warning",
    pending: "badge-info",
    failed: "badge-danger",
    terminated: "badge-info",
  };
  return <span className={`badge ${map[s] ?? "badge-info"}`}>{s}</span>;
};

export function PreviewManagementPage() {
  const queryClient = useQueryClient();

  // In a real app, project_id comes from a selector; hardcode for skeleton
  const { data: envs = [] } = useQuery<PreviewEnv[]>({
    queryKey: ["previews"],
    queryFn: async () => [],
    refetchInterval: 10_000,
  });

  const restartMutation = useMutation({
    mutationFn: async (envId: string) => {
      await axios.post(`/api/v1/preview/${envId}/restart`);
    },
    onSuccess: () => {
      toast.success("重启已排队");
      queryClient.invalidateQueries({ queryKey: ["previews"] });
    },
    onError: () => toast.error("重启失败"),
  });

  return (
    <div>
      <h1 style={{ fontSize: 20, fontWeight: 700, marginBottom: 20 }}>预览管理</h1>
      {envs.length === 0 ? (
        <p style={{ color: "var(--text-muted)" }}>当前无活跃预览环境</p>
      ) : (
        <table style={{ width: "100%", borderCollapse: "collapse" }}>
          <thead>
            <tr style={{ borderBottom: "1px solid var(--border)", color: "var(--text-muted)", fontSize: 12, textAlign: "left" }}>
              <th style={{ padding: "8px 12px" }}>类型</th>
              <th style={{ padding: "8px 12px" }}>状态</th>
              <th style={{ padding: "8px 12px" }}>预览 URL</th>
              <th style={{ padding: "8px 12px" }}>操作</th>
            </tr>
          </thead>
          <tbody>
            {envs.map((env) => (
              <tr key={env.id} style={{ borderBottom: "1px solid var(--border)" }}>
                <td style={{ padding: "10px 12px" }}>{env.env_type}</td>
                <td style={{ padding: "10px 12px" }}>{statusBadge(env.status)}</td>
                <td style={{ padding: "10px 12px" }}>
                  {env.preview_url ? (
                    <a href={env.preview_url} target="_blank" rel="noreferrer">{env.preview_url}</a>
                  ) : "—"}
                </td>
                <td style={{ padding: "10px 12px" }}>
                  {env.status === "ready" && (
                    <button
                      className="btn-ghost"
                      style={{ fontSize: 12 }}
                      onClick={() => restartMutation.mutate(env.id)}
                      disabled={restartMutation.isPending}
                    >
                      重启容器
                    </button>
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
