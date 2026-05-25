import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import axios from "axios";
import toast from "react-hot-toast";
import { useSystemStatus } from "../hooks/useSystemStatus";

interface Metrics {
  avg_time_to_merge_minutes: number | null;
  avg_time_to_merge_target: number;
  first_pass_approval_rate: number | null;
  first_pass_target: number;
  preview_success_rate: number | null;
  preview_success_target: number;
  pending_review_1: number;
  pending_review_2: number;
  total_reviews_completed: number;
}

const STAGE_LABEL = { normal: "正常运行", throttled: "单线程降速", paused: "已暂停" } as const;
const STAGE_COLOR = { normal: "var(--success)", throttled: "var(--warning)", paused: "var(--danger)" } as const;

function formatMinutes(m: number | null): string {
  if (m === null) return "—";
  const h = Math.floor(m / 60);
  const min = Math.round(m % 60);
  return h > 0 ? `${h}h ${min}m` : `${min}m`;
}

function MetricRow({
  label,
  value,
  target,
  pass,
  targetLabel,
}: {
  label: string;
  value: string;
  target: string;
  pass: boolean | null;
  targetLabel: string;
}) {
  return (
    <div style={{
      display: "grid",
      gridTemplateColumns: "1fr auto auto auto",
      gap: 16,
      padding: "12px 0",
      borderBottom: "1px solid var(--border)",
      alignItems: "center",
      fontSize: 14,
    }}>
      <span style={{ color: "var(--muted)" }}>{label}</span>
      <span style={{ fontWeight: 600, textAlign: "right" }}>{value}</span>
      <span style={{ color: "var(--muted)", fontSize: 12, textAlign: "right" }}>{targetLabel} {target}</span>
      {pass === null ? (
        <span style={{ color: "var(--muted)", fontSize: 12 }}>—</span>
      ) : pass ? (
        <span style={{ color: "var(--success)", fontWeight: 600 }}>✓</span>
      ) : (
        <span style={{ color: "var(--danger)", fontSize: 12 }}>✗ 待改善</span>
      )}
    </div>
  );
}

export function SystemStatusPage() {
  const queryClient = useQueryClient();
  const { data: status, isLoading: statusLoading } = useSystemStatus();
  const { data: metrics, isLoading: metricsLoading } = useQuery<Metrics>({
    queryKey: ["system-metrics"],
    queryFn: async () => (await axios.get("/api/v1/system/metrics")).data,
    refetchInterval: 60_000,
  });

  const [maxSlots, setMaxSlots] = useState<number | null>(null);
  const [pauseThreshold, setPauseThreshold] = useState<number | null>(null);

  const saveConfig = useMutation({
    mutationFn: async () =>
      axios.post("/api/v1/system/config", {
        max_slots: maxSlots ?? status?.concurrency.max_slots,
        pause_threshold: pauseThreshold ?? status?.concurrency.pause_threshold,
      }),
    onSuccess: () => {
      toast.success("配置已保存");
      queryClient.invalidateQueries({ queryKey: ["system-status"] });
      setMaxSlots(null);
      setPauseThreshold(null);
    },
    onError: () => toast.error("保存失败"),
  });

  if (statusLoading) return <div className="empty-state"><div className="spinner" /></div>;
  if (!status) return <div className="empty-state"><p>无法获取系统状态</p></div>;

  const c = status.concurrency;
  const stage = c.stage as keyof typeof STAGE_LABEL;
  const stageColor = STAGE_COLOR[stage] ?? "var(--muted)";

  const backpressurePct = Math.min(100, Math.round((c.pending_review_count / c.pause_threshold) * 100));
  const currentMaxSlots = maxSlots ?? c.max_slots;
  const currentPauseThreshold = pauseThreshold ?? c.pause_threshold;
  const configDirty = maxSlots !== null || pauseThreshold !== null;

  // Build slot list: active slots shown as occupied, rest as idle
  const slots = Array.from({ length: c.max_slots }, (_, i) => i < c.active_slots);

  return (
    <div style={{ maxWidth: 720 }}>
      <div className="page-header">
        <h1>系统状态</h1>
      </div>

      {/* Backpressure banner */}
      {stage === "paused" && (
        <div style={{
          background: "rgba(248,113,113,0.12)",
          border: "1px solid rgba(248,113,113,0.4)",
          borderRadius: 8,
          padding: "12px 16px",
          marginBottom: 16,
          display: "flex",
          alignItems: "center",
          gap: 10,
          fontSize: 13,
        }}>
          <span style={{ color: "var(--danger)", fontWeight: 600 }}>系统已暂停</span>
          <span style={{ color: "var(--muted)" }}>
            待审核积压已达上限（{c.pending_review_count} / {c.pause_threshold}）。请完成积压审核，系统将自动恢复。外部输入已进入暂存队列，不会丢失。
          </span>
        </div>
      )}
      {stage === "throttled" && (
        <div style={{
          background: "rgba(234,179,8,0.1)",
          border: "1px solid rgba(234,179,8,0.35)",
          borderRadius: 8,
          padding: "12px 16px",
          marginBottom: 16,
          fontSize: 13,
          color: "var(--warning)",
          fontWeight: 500,
        }}>
          系统降速中 — 并发已限制为单线程，待审核积压 {c.pending_review_count} 条
        </div>
      )}

      {/* Concurrency card */}
      <div className="card" style={{ padding: 20, marginBottom: 16 }}>
        <div className="section-label" style={{ marginBottom: 14 }}>并发状态</div>

        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 18 }}>
          <div style={{ width: 12, height: 12, borderRadius: "50%", background: stageColor, flexShrink: 0 }} />
          <span style={{ fontWeight: 600, fontSize: 15, color: stageColor }}>{STAGE_LABEL[stage] ?? stage}</span>
        </div>

        {/* Slot dots */}
        <div style={{ display: "flex", gap: 6, marginBottom: 12 }}>
          {slots.map((occupied, i) => (
            <div
              key={i}
              title={occupied ? `槽位 ${i + 1} 占用中` : `槽位 ${i + 1} 空闲`}
              style={{
                width: 16,
                height: 16,
                borderRadius: "50%",
                background: occupied ? "var(--purple)" : "var(--border)",
                cursor: "default",
              }}
            />
          ))}
          <span style={{ marginLeft: 8, fontSize: 13, color: "var(--muted)", alignSelf: "center" }}>
            {c.active_slots} / {c.max_slots} 占用
          </span>
        </div>

        {/* Slot detail list */}
        <div style={{ fontSize: 13, borderTop: "1px solid var(--border)", paddingTop: 12 }}>
          {slots.map((occupied, i) => (
            <div key={i} style={{ display: "flex", alignItems: "center", gap: 10, padding: "5px 0" }}>
              <div style={{
                width: 8, height: 8, borderRadius: "50%", flexShrink: 0,
                background: occupied ? "var(--purple)" : "var(--border)",
              }} />
              <span style={{ color: "var(--muted)", minWidth: 48 }}>槽位 {i + 1}</span>
              {occupied ? (
                <span style={{ color: "var(--text)" }}>占用中</span>
              ) : (
                <span style={{ color: "var(--muted)" }}>空闲</span>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* Backpressure monitor */}
      <div className="card" style={{ padding: 20, marginBottom: 16 }}>
        <div className="section-label" style={{ marginBottom: 12 }}>积压监控</div>
        <div style={{ display: "flex", justifyContent: "space-between", fontSize: 13, marginBottom: 8 }}>
          <span style={{ color: "var(--muted)" }}>待审核积压</span>
          <span style={{ fontWeight: 600 }}>
            {c.pending_review_count} 条 / 暂停阈值 {c.pause_threshold} 条
          </span>
        </div>
        <div style={{ background: "var(--border)", borderRadius: 4, height: 8, overflow: "hidden" }}>
          <div style={{
            height: "100%",
            borderRadius: 4,
            width: `${backpressurePct}%`,
            background: backpressurePct >= 100
              ? "var(--danger)"
              : backpressurePct >= 60
                ? "var(--warning)"
                : "var(--purple)",
            transition: "width 0.4s",
          }} />
        </div>
        <div style={{ fontSize: 11, color: "var(--muted)", marginTop: 4, textAlign: "right" }}>
          {backpressurePct}%
        </div>
      </div>

      {/* KPI metrics */}
      <div className="card" style={{ padding: 20, marginBottom: 16 }}>
        <div className="section-label" style={{ marginBottom: 4 }}>成功指标（过去 30 天）</div>
        {metricsLoading ? (
          <div style={{ padding: "20px 0", display: "flex", justifyContent: "center" }}>
            <div className="spinner" />
          </div>
        ) : metrics ? (
          <>
            <MetricRow
              label="需求→合并平均时长"
              value={formatMinutes(metrics.avg_time_to_merge_minutes)}
              targetLabel="目标 <"
              target={formatMinutes(metrics.avg_time_to_merge_target)}
              pass={metrics.avg_time_to_merge_minutes !== null
                ? metrics.avg_time_to_merge_minutes <= metrics.avg_time_to_merge_target
                : null}
            />
            <MetricRow
              label="首次审核通过率"
              value={metrics.first_pass_approval_rate !== null
                ? `${Math.round(metrics.first_pass_approval_rate * 100)}%`
                : "—"}
              targetLabel="目标 >"
              target={`${Math.round(metrics.first_pass_target * 100)}%`}
              pass={metrics.first_pass_approval_rate !== null
                ? metrics.first_pass_approval_rate >= metrics.first_pass_target
                : null}
            />
            <MetricRow
              label="预览启动成功率"
              value={metrics.preview_success_rate !== null
                ? `${(metrics.preview_success_rate * 100).toFixed(1)}%`
                : "—"}
              targetLabel="目标 >"
              target={`${Math.round(metrics.preview_success_target * 100)}%`}
              pass={metrics.preview_success_rate !== null
                ? metrics.preview_success_rate >= metrics.preview_success_target
                : null}
            />
            <div style={{ display: "flex", justifyContent: "space-between", padding: "12px 0", fontSize: 13 }}>
              <span style={{ color: "var(--muted)" }}>已完成审核总计</span>
              <span style={{ fontWeight: 600 }}>{metrics.total_reviews_completed} 次</span>
            </div>
          </>
        ) : (
          <p style={{ color: "var(--muted)", fontSize: 13 }}>指标加载失败</p>
        )}
      </div>

      {/* Config sliders */}
      <div className="card" style={{ padding: 20 }}>
        <div className="section-label" style={{ marginBottom: 16 }}>并发配置</div>

        <div style={{ marginBottom: 16 }}>
          <div style={{ display: "flex", justifyContent: "space-between", fontSize: 13, marginBottom: 6 }}>
            <label style={{ color: "var(--muted)" }}>最大并发槽位</label>
            <span style={{ fontWeight: 600 }}>{currentMaxSlots}</span>
          </div>
          <input
            type="range"
            min={1}
            max={20}
            value={currentMaxSlots}
            onChange={e => setMaxSlots(Number(e.target.value))}
            style={{ width: "100%" }}
          />
          <div style={{ display: "flex", justifyContent: "space-between", fontSize: 11, color: "var(--muted)" }}>
            <span>1</span><span>20</span>
          </div>
        </div>

        <div style={{ marginBottom: 20 }}>
          <div style={{ display: "flex", justifyContent: "space-between", fontSize: 13, marginBottom: 6 }}>
            <label style={{ color: "var(--muted)" }}>暂停阈值（待审核积压上限）</label>
            <span style={{ fontWeight: 600 }}>{currentPauseThreshold}</span>
          </div>
          <input
            type="range"
            min={5}
            max={100}
            value={currentPauseThreshold}
            onChange={e => setPauseThreshold(Number(e.target.value))}
            style={{ width: "100%" }}
          />
          <div style={{ display: "flex", justifyContent: "space-between", fontSize: 11, color: "var(--muted)" }}>
            <span>5</span><span>100</span>
          </div>
        </div>

        <div style={{ display: "flex", justifyContent: "flex-end" }}>
          <button
            className="btn btn-primary btn-sm"
            disabled={!configDirty || saveConfig.isPending}
            onClick={() => saveConfig.mutate()}
          >
            {saveConfig.isPending ? "保存中…" : "保存配置"}
          </button>
        </div>
      </div>
    </div>
  );
}
