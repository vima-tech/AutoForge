import { Routes, Route, NavLink } from "react-router-dom";
import { IssueQueuePage } from "./pages/IssueQueuePage";
import { Review1Page } from "./pages/Review1Page";
import { Review2Page } from "./pages/Review2Page";
import { PreviewManagementPage } from "./pages/PreviewManagementPage";
import { SystemStatusPage } from "./pages/SystemStatusPage";
import { useSystemStatus } from "./hooks/useSystemStatus";

export default function App() {
  const { data: status } = useSystemStatus();
  const stage = status?.concurrency?.stage;

  return (
    <div style={{ display: "flex", height: "100vh" }}>
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
