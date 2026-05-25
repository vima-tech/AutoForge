import { useState } from "react";

interface Props {
  url: string | null;
  label: string;
  indicator: "red" | "green" | "blue";
  status: string;
  onRestart?: () => void;
}

const COLORS = { red: "#f87171", green: "#4ade80", blue: "#6c8ef5" };

export function PreviewFrame({ url, label, indicator, status, onRestart }: Props) {
  const [fullscreen, setFullscreen] = useState(false);
  const color = COLORS[indicator];

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        border: "1px solid var(--border)",
        borderRadius: 8,
        overflow: "hidden",
        height: fullscreen ? "100vh" : 320,
        position: fullscreen ? "fixed" : "relative",
        inset: fullscreen ? 0 : undefined,
        zIndex: fullscreen ? 9999 : undefined,
        background: "var(--surface)",
      }}
    >
      {/* Header */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "8px 12px",
          borderBottom: "1px solid var(--border)",
          background: "var(--elevated)",
        }}
      >
        <span style={{ width: 10, height: 10, borderRadius: "50%", background: color, display: "inline-block" }} />
        <span style={{ fontSize: 12, fontWeight: 600, flex: 1 }}>{label}</span>
        {url && (
          <>
            <a href={url} target="_blank" rel="noreferrer" style={{ fontSize: 11 }}>↗</a>
            <button className="btn btn-ghost btn-sm" onClick={() => setFullscreen(!fullscreen)}>
              {fullscreen ? "收起" : "⤢"}
            </button>
          </>
        )}
        {onRestart && status === "ready" && (
          <button className="btn btn-ghost btn-sm" onClick={onRestart}>↺</button>
        )}
      </div>

      {/* Content */}
      <div style={{ flex: 1, overflow: "hidden", position: "relative" }}>
        {status === "building" || status === "pending" ? (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              justifyContent: "center",
              height: "100%",
              gap: 12,
              color: "var(--muted)",
            }}
          >
            <div className="spinner" style={{ width: 24, height: 24 }} />
            <span style={{ fontSize: 13 }}>
              {status === "building" ? "构建预览中…" : "等待构建…"}
            </span>
          </div>
        ) : status === "failed" ? (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              justifyContent: "center",
              height: "100%",
              gap: 8,
              color: "var(--red)",
              fontSize: 13,
            }}
          >
            <span style={{ fontSize: 24 }}>⚠</span>
            <span>预览构建失败</span>
            {onRestart && (
              <button className="btn btn-ghost btn-sm" onClick={onRestart}>重新构建</button>
            )}
          </div>
        ) : url ? (
          <iframe
            src={url}
            style={{ width: "100%", height: "100%", border: "none" }}
            title={label}
            sandbox="allow-scripts allow-same-origin allow-forms"
          />
        ) : (
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              height: "100%",
              color: "var(--muted)",
              fontSize: 13,
            }}
          >
            无预览 URL
          </div>
        )}
      </div>
    </div>
  );
}
