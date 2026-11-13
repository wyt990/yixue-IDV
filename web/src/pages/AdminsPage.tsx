import { useEffect, useState } from "react";
import { api, type Admin } from "../api";

export function AdminsPage() {
  const [admins, setAdmins] = useState<Admin[]>([]);
  const [error, setError] = useState("");

  useEffect(() => {
    api.admins
      .list()
      .then(setAdmins)
      .catch((err) => setError(err.message));
  }, []);

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div>
        <h1>管理员</h1>
        <p className="muted">阶段 1 支持查看与 API 创建；界面创建可后续补。</p>
      </div>
      {error ? <p className="error">{error}</p> : null}
      <div className="card">
        <table>
          <thead>
            <tr>
              <th>用户名</th>
              <th>显示名</th>
              <th>创建时间</th>
            </tr>
          </thead>
          <tbody>
            {admins.map((a) => (
              <tr key={a.id}>
                <td className="mono">{a.username}</td>
                <td>{a.display_name}</td>
                <td className="muted">{new Date(a.created_at).toLocaleString()}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
