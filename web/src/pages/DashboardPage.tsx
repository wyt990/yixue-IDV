import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import {
  api,
  type DashboardOverview,
} from "../api";
import { PageHeader } from "../components/PageHeader";
import { useMessage } from "../message";
import { taskKindLabel, taskStateLabel } from "../taskLabels";

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
      <PageHeader
        title="工作台"
        meta={
          stats
            ? `边缘在线 ${stats.edges_online}/${stats.edges_total} · 进行中任务 ${stats.tasks_running}`
            : "正在汇总"
        }
      />

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
          <h2>离线边缘</h2>
          <Link className="btn ghost sm" to="/edges">
            查看全部
          </Link>
        </div>
        <table className="data-table">
          <thead>
            <tr>
              <th>名称</th>
              <th>状态</th>
            </tr>
          </thead>
          <tbody>
            {!data || data.edges.length === 0 ? (
              <tr>
                <td colSpan={2} className="muted">
                  暂无边缘节点
                </td>
              </tr>
            ) : data.edges.every((e) => e.online) ? (
              <tr>
                <td colSpan={2} className="muted">
                  边缘节点都在线
                </td>
              </tr>
            ) : (
              data.edges
                .filter((e) => !e.online)
                .map((e) => (
                  <tr key={e.id}>
                    <td>{e.name}</td>
                    <td>
                      <span className="status">
                        <span className="status-dot off" />
                        离线
                      </span>
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
            <Link className="btn ghost sm" to="/tasks">
              维护任务
            </Link>
          </div>
          <table className="data-table">
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
          <table className="data-table">
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
          <Link className="btn ghost sm" to="/labs">
            校区 / 机房
          </Link>
        </div>
        <table className="data-table">
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
                  <td>
                    <Link to={`/clients?lab=${encodeURIComponent(lab.id)}`}>{lab.name}</Link>
                  </td>
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
