import { useState } from "react";
import { Routes, Route, NavLink } from "react-router-dom";
import { IssueQueuePage } from "./pages/IssueQueuePage";
import { Review1Page } from "./pages/Review1Page";
import { Review2Page } from "./pages/Review2Page";
import { PreviewManagementPage } from "./pages/PreviewManagementPage";
import { SystemStatusPage } from "./pages/SystemStatusPage";
import { useSystemStatus } from "./hooks/useSystemStatus";
import { isConfigured, setAdminCredentials } from "./store/auth";

function SetupModal({ onDone }: { onDone: () => void }) {
  const [id, setId] = useState("admin");
  const [key, setKey] = useState("");
  return (
    <div style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.7)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 9999 }}>
      <div className="card" style={{ padding: 28, width: 420 }}>
        <h3 style={{ fontWeight: 700, marginBottom: 6 }}>设置管理员凭据</h3>
        <p style={{ fontSize: 13, color: "var(--muted)", marginBottom: 20 }}>
          凭据存储在浏览器 localStorage 中，每次 API 调用自动附带。若未设置 <code>ADMIN_API_KEY</code> 环境变量，留空 Key 字段仍可使用。
        </p>
        <label style={{ fontSize: 12, color: "var(--muted)" }}>管理员 ID（显示用）</label>
        <input value={id} onChange={e => setId(e.target.value)} style={{ marginBottom: 12 }} />
        <label style={{ fontSize: 12, color: "var(--muted)" }}>Admin API Key（ADMIN_API_KEY）</label>
        <input
          type="password"
          value={key}
          onChange={e => setKey(e.target.value)}
          placeholder="留空则不做鉴权"
          style={{ marginBottom: 20 }}
        />
        <div style={{ display: "flex", justifyContent: "flex-end" }}>
          <button
            className="btn btn-primary"
            onClick={() => { setAdminCredentials(id.trim() || "admin", key.trim()); onDone(); }}
          >
            确认
          </button>
        </div>
      </div>
    </div>
  );
}

export default function App() {
  const [setupDone, setSetupDone] = useState(isConfigured());
  const { data: status } = useSystemStatus();
  const stage = status?.concurrency?.stage;

  return (
    <div style={{ display: "flex", height: "100vh" }}>
      {!setupDone && <SetupModal onDone={() => setSetupDone(true)} />}

      <nav style={{
        width: 220, background: "var(--surface)", borderRight: "1px solid var(--border)",
        display: "flex", flexDirection: "column", padding: "20px 0",
      }}>
        <div style={{ padding: "0 20px 20px", fontWeight: 700, fontSize: 16 }}>
          AutoForge
        </div>
        {stage && stage !== "normal" && (
          <div style={{
            margin: "0 12px 16px", padding: "8px 12px", borderRadius: 6,
            background: stage === "paused" ? "rgba(248,113,113,0.15)" : "rgba(251,191,36,0.15)",
            color: stage === "paused" ? "var(--danger)" : "var(--warning)",
            fontSize: 12, fontWeight: 600,
          }}>
            {stage === "paused" ? "系统已暂停" : "单线程降速中"}
          </div>
        )}
        {[
          { to: "/", label: "需求队列" },
          { to: "/review-1", label: "审核节点 1" },
          { to: "/review-2", label: "审核节点 2" },
          { to: "/preview", label: "预览管理" },
          { to: "/system", label: "系统状态" },
        ].map(({ to, label }) => (
          <NavLink
            key={to}
            to={to}
            end={to === "/"}
            style={({ isActive }) => ({
              padding: "10px 20px",
              color: isActive ? "var(--accent)" : "var(--text-muted)",
              background: isActive ? "rgba(108,142,245,0.08)" : "transparent",
              fontWeight: isActive ? 600 : 400,
              display: "block",
            })}
          >
            {label}
          </NavLink>
        ))}
        <div style={{ marginTop: "auto", padding: "0 20px" }}>
          <button
            className="btn btn-ghost btn-sm"
            style={{ width: "100%", fontSize: 11 }}
            onClick={() => { localStorage.removeItem("autoforge_admin_key"); setSetupDone(false); }}
          >
            重置凭据
          </button>
        </div>
      </nav>
      <main style={{ flex: 1, overflow: "auto", padding: 24 }}>
        <Routes>
          <Route path="/" element={<IssueQueuePage />} />
          <Route path="/review-1/:issueId?" element={<Review1Page />} />
          <Route path="/review-2/:crId?" element={<Review2Page />} />
          <Route path="/preview" element={<PreviewManagementPage />} />
          <Route path="/system" element={<SystemStatusPage />} />
        </Routes>
      </main>
    </div>
  );
}
