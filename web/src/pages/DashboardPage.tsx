import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import {
  api,
  type DashboardOverview,
} from "../api";
import { useMessage } from "../message";
import { taskKindLabel, taskStateLabel } from "../taskLabels";

function formatBytes(n: number) {
  if (!Number.isFinite(n) || n <= 0) return "0 B";
  if (n < 1024) return `${Math.round(n)} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatRate(bps: number) {
  if (!Number.isFinite(bps) || bps <= 0) return "0 B/s";
  if (bps < 1024) return `${Math.round(bps)} B/s`;
  if (bps < 1024 * 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${(bps / (1024 * 1024)).toFixed(1)} MB/s`;
}

function formatHostLoad(online: boolean, cpu: number, memUsed: number, memTotal: number, rx: number, tx: number) {
  if (!online) return "—";
  const mem =
    memTotal > 0
      ? `${formatBytes(memUsed)}/${formatBytes(memTotal)}`
      : formatBytes(memUsed);
  return `CPU ${cpu.toFixed(0)}% · 内存 ${mem} · ↓${formatRate(rx)} / ↑${formatRate(tx)}`;
}

export function DashboardPage() {
  const msg = useMessage();
  const [data, setData] = useState<DashboardOverview | null>(null);

  async function refresh() {
    setData(await api.dashboard());
  }

  useEffect(() => {
    refresh().catch((err) => msg.error(err.message));
    const timer = setInterval(() => {
      refresh().catch(() => undefined);
    }, 3000);
    return () => clearInterval(timer);
  }, []);

  const stats = data?.stats;

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div className="panel-head">
        <div>
          <h1>工作台</h1>
          <p className="muted">资产规模、边缘负载与任务动态（约 3 秒自动刷新）。</p>
        </div>
        <div className="page-actions">
          <Link className="btn" to="/tasks">
            采集 / 部署
          </Link>
          <Link className="btn ghost" to="/clients">
            终端
          </Link>
          <Link className="btn ghost" to="/edges">
            边缘
          </Link>
        </div>
      </div>

      <div className="grid stats stats-6">
        {[
          ["校区", stats?.campuses],
          ["机房", stats?.labs],
          ["终端", stats?.clients],
          ["镜像", stats?.images],
          [
            "Edge 在线",
            stats
              ? `${stats.edges_online}/${stats.edges_total}`
              : undefined,
          ],
          ["进行中任务", stats?.tasks_running],
        ].map(([label, value]) => (
          <div className="card" key={String(label)}>
            <div className="stat-label">{label}</div>
            <div className="stat-value mono">{value ?? "—"}</div>
          </div>
        ))}
      </div>

      <div className="card">
        <div className="panel-head">
          <h2>边缘节点</h2>
          <Link className="btn ghost" to="/edges">
            查看全部
          </Link>
        </div>
        <table>
          <thead>
            <tr>
              <th>名称</th>
              <th>状态</th>
              <th>节点负载</th>
            </tr>
          </thead>
          <tbody>
            {!data || data.edges.length === 0 ? (
              <tr>
                <td colSpan={3} className="muted">
                  暂无边缘节点
                </td>
              </tr>
            ) : (
              data.edges.map((e) => (
                <tr key={e.id}>
                  <td>{e.name}</td>
                  <td>
                    <span className="tag">{e.online ? "在线" : "离线"}</span>
                  </td>
                  <td className="mono muted">
                    {formatHostLoad(
                      e.online,
                      e.cpu_pct,
                      e.mem_used_bytes,
                      e.mem_total_bytes,
                      e.net_rx_bps,
                      e.net_tx_bps,
                    )}
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      <div className="form-row" style={{ alignItems: "stretch" }}>
        <div className="card" style={{ flex: 1 }}>
          <div className="panel-head">
            <h2>进行中任务</h2>
            <Link className="btn ghost" to="/tasks">
              维护任务
            </Link>
          </div>
          <table>
            <thead>
              <tr>
                <th>名称</th>
                <th>类型</th>
                <th>状态</th>
                <th>进度</th>
                <th>更新</th>
              </tr>
            </thead>
            <tbody>
              {!data || data.tasks_active.length === 0 ? (
                <tr>
                  <td colSpan={5} className="muted">
                    当前无进行中任务
                  </td>
                </tr>
              ) : (
                data.tasks_active.map((t) => (
                  <tr key={t.id}>
                    <td>{t.name}</td>
                    <td>{taskKindLabel(t.kind)}</td>
                    <td>{taskStateLabel(t.state)}</td>
                    <td className="mono">{t.progress_text || "—"}</td>
                    <td className="muted">
                      {new Date(t.updated_at).toLocaleString()}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>

        <div className="card" style={{ flex: 1 }}>
          <div className="panel-head">
            <h2>最近失败</h2>
          </div>
          <table>
            <thead>
              <tr>
                <th>名称</th>
                <th>类型</th>
                <th>错误</th>
                <th>时间</th>
              </tr>
            </thead>
            <tbody>
              {!data || data.tasks_failed.length === 0 ? (
                <tr>
                  <td colSpan={4} className="muted">
                    暂无失败记录
                  </td>
                </tr>
              ) : (
                data.tasks_failed.map((t) => (
                  <tr key={t.id}>
                    <td>{t.name}</td>
                    <td>{taskKindLabel(t.kind)}</td>
                    <td className="error" title={t.error_message}>
                      {t.error_message || taskStateLabel(t.state)}
                    </td>
                    <td className="muted">
                      {new Date(t.updated_at).toLocaleString()}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </div>

      <div className="card">
        <div className="panel-head">
          <h2>机房摘要</h2>
          <Link className="btn ghost" to="/labs">
            校区 / 机房
          </Link>
        </div>
        <table>
          <thead>
            <tr>
              <th>机房</th>
              <th>校区</th>
              <th>终端</th>
              <th>默认镜像</th>
            </tr>
          </thead>
          <tbody>
            {!data || data.labs.length === 0 ? (
              <tr>
                <td colSpan={4} className="muted">
                  暂无机房
                </td>
              </tr>
            ) : (
              data.labs.map((lab) => (
                <tr key={lab.id}>
                  <td>{lab.name}</td>
                  <td>{lab.campus_name}</td>
                  <td className="mono">{lab.client_count}</td>
                  <td className="muted">{lab.default_image_name}</td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
