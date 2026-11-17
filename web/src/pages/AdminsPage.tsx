import { useEffect, useState } from "react";
import { api, getToken, type Admin } from "../api";

export function AdminsPage() {
  const [admins, setAdmins] = useState<Admin[]>([]);
  const [error, setError] = useState("");

  useEffect(() => {
    api.admins
      .list()
      .then(setAdmins)
      .catch((err) => setError(err.message));
  }, []);

  function exportAudit() {
    const token = getToken();
    fetch(api.phase3.auditExportUrl, {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
    })
      .then(async (res) => {
        if (!res.ok) throw new Error(await res.text());
        const blob = await res.blob();
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = "idv-audit.csv";
        a.click();
        URL.revokeObjectURL(url);
      })
      .catch((err) => setError(String(err.message || err)));
  }

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div className="panel-head">
        <div>
          <h1>管理员</h1>
          <p className="muted">阶段 1 支持查看与 API 创建；界面创建可后续补。</p>
        </div>
        <div className="page-actions">
          <button className="btn ghost" type="button" onClick={exportAudit}>
            导出审计 CSV
          </button>
        </div>
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
