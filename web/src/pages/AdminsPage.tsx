import { FormEvent, useEffect, useState } from "react";
import { api, getToken, type Admin } from "../api";
import { Modal } from "../components/Modal";
import { PageHeader } from "../components/PageHeader";
import { useAuth } from "../auth";
import { useMessage } from "../message";

const ROLES = [
  { id: "super", label: "超级管理员" },
  { id: "lab_admin", label: "机房管理员" },
  { id: "readonly", label: "只读" },
] as const;

function roleLabel(role?: string) {
  return ROLES.find((r) => r.id === role)?.label || role || "—";
}

export function AdminsPage() {
  const msg = useMessage();
  const { user, setUser } = useAuth();
  const superUser = user?.role === "super";
  const [admins, setAdmins] = useState<Admin[]>([]);
  const [error, setError] = useState("");
  const [formError, setFormError] = useState("");
  const [editing, setEditing] = useState<Admin | null>(null);
  const [open, setOpen] = useState(false);
  const [username, setUsername] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [role, setRole] = useState("lab_admin");
  const [password, setPassword] = useState("");

  async function refresh() {
    setAdmins(await api.admins.list());
  }

  useEffect(() => {
    refresh().catch((err) => setError(err.message));
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

  function openCreate() {
    setEditing(null);
    setUsername("");
    setDisplayName("");
    setRole("lab_admin");
    setPassword("");
    setFormError("");
    setOpen(true);
  }

  function openEdit(admin: Admin) {
    setEditing(admin);
    setUsername(admin.username);
    setDisplayName(admin.display_name);
    setRole(admin.role || "readonly");
    setPassword("");
    setFormError("");
    setOpen(true);
  }

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setFormError("");
    try {
      const saved = editing
        ? await api.admins.update(editing.id, {
            username,
            display_name: displayName,
            role,
            password,
          })
        : await api.admins.create({
            username,
            password,
            display_name: displayName,
            role,
          });
      if (saved.id === user?.id) setUser(saved);
      setOpen(false);
      await refresh();
      msg.success(editing ? "已更新管理员" : "已添加管理员");
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "保存失败");
    }
  }

  async function remove(admin: Admin) {
    if (!window.confirm(`删除管理员 ${admin.username}？`)) return;
    try {
      await api.admins.delete(admin.id);
      await refresh();
      msg.success("已删除管理员");
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "删除失败");
    }
  }

  return (
    <div className="grid" style={{ gap: "0.75rem" }}>
      <PageHeader title="管理员" meta={`${admins.length} 个账号`}>
        <button className="btn ghost sm" type="button" onClick={exportAudit}>
          导出审计
        </button>
        {superUser ? (
          <button className="btn sm" type="button" onClick={openCreate}>
            添加
          </button>
        ) : null}
      </PageHeader>
      {error ? <p className="error">{error}</p> : null}
      {!superUser ? (
        <p className="muted">当前账号只能查看。增删改需要超级管理员。</p>
      ) : null}
      <div className="card">
        <table className="data-table">
          <thead>
            <tr>
              <th>用户名</th>
              <th>显示名</th>
              <th>角色</th>
              <th>创建时间</th>
              {superUser ? <th>操作</th> : null}
            </tr>
          </thead>
          <tbody>
            {admins.length === 0 ? (
              <tr>
                <td colSpan={superUser ? 5 : 4} className="muted">
                  暂无管理员
                </td>
              </tr>
            ) : (
              admins.map((a) => (
                <tr key={a.id}>
                  <td className="mono">{a.username}</td>
                  <td>{a.display_name}</td>
                  <td>{roleLabel(a.role)}</td>
                  <td className="muted">{new Date(a.created_at).toLocaleString()}</td>
                  {superUser ? (
                    <td>
                      <div className="row-actions">
                        <button type="button" className="btn ghost sm" onClick={() => openEdit(a)}>
                          编辑
                        </button>
                        <button
                          type="button"
                          className="btn danger sm"
                          disabled={a.id === user?.id}
                          title={a.id === user?.id ? "不能删除当前登录账号" : undefined}
                          onClick={() => remove(a)}
                        >
                          删除
                        </button>
                      </div>
                    </td>
                  ) : null}
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      <Modal
        open={open}
        title={editing ? "编辑管理员" : "添加管理员"}
        onClose={() => setOpen(false)}
      >
        <form onSubmit={onSubmit}>
          {formError ? <p className="error">{formError}</p> : null}
          <div className="field">
            <label>用户名</label>
            <input value={username} onChange={(e) => setUsername(e.target.value)} required />
          </div>
          <div className="field">
            <label>显示名</label>
            <input
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              placeholder="留空则与用户名相同"
            />
          </div>
          <div className="field">
            <label>角色</label>
            <select value={role} onChange={(e) => setRole(e.target.value)}>
              {ROLES.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.label}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>{editing ? "新密码（留空则不修改）" : "密码"}</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required={!editing}
              minLength={editing && !password ? undefined : 6}
              autoComplete="new-password"
            />
          </div>
          <div className="modal-actions">
            <button type="button" className="btn ghost" onClick={() => setOpen(false)}>
              取消
            </button>
            <button className="btn" type="submit">
              保存
            </button>
          </div>
        </form>
      </Modal>
    </div>
  );
}
