import { useEffect, useState } from "react";
import { api } from "../api";

type Row = {
  id: string;
  edge_id: string;
  mac: string;
  source: string;
  status: string;
  message: string;
  last_seen: string;
};

export function PeSessionsPage() {
  const [rows, setRows] = useState<Row[]>([]);
  const [error, setError] = useState("");

  useEffect(() => {
    const load = () =>
      api.peSessions
        .list()
        .then(setRows)
        .catch((err) => setError(err.message));
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  }, []);

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div>
        <h1>PE 会话</h1>
        <p className="muted">
          客户端经 PXE/iPXE 或 pe-agent 进入维护环境后的报到记录。
        </p>
      </div>
      {error ? <p className="error">{error}</p> : null}
      <div className="card">
        <table>
          <thead>
            <tr>
              <th>MAC</th>
              <th>来源</th>
              <th>状态</th>
              <th>消息</th>
              <th>最近见到</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.id}>
                <td className="mono">{r.mac}</td>
                <td>{r.source}</td>
                <td>
                  <span className="tag">{r.status}</span>
                </td>
                <td className="muted">{r.message}</td>
                <td className="muted">
                  {new Date(r.last_seen).toLocaleString()}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
