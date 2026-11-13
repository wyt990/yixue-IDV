import { useEffect, useState } from "react";
import { api, type DashboardStats } from "../api";

export function DashboardPage() {
  const [stats, setStats] = useState<DashboardStats | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    api
      .dashboard()
      .then(setStats)
      .catch((err) => setError(err.message));
  }, []);

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div>
        <h1>工作台</h1>
        <p className="muted">一看清资产规模；Edge / 任务指标在后续阶段接入。</p>
      </div>
      {error ? <p className="error">{error}</p> : null}
      <div className="grid stats">
        {[
          ["校区", stats?.campuses],
          ["机房", stats?.labs],
          ["终端", stats?.clients],
          ["镜像", stats?.images],
        ].map(([label, value]) => (
          <div className="card" key={String(label)}>
            <div className="stat-label">{label}</div>
            <div className="stat-value mono">{value ?? "—"}</div>
          </div>
        ))}
      </div>
      <div className="card">
        <h2>状态</h2>
        <p className="muted" style={{ marginBottom: 0 }}>
          Edge 在线 {stats?.edges_online ?? 0} · 进行中任务{" "}
          {stats?.tasks_running ?? 0}（阶段 2+）
        </p>
      </div>
    </div>
  );
}
