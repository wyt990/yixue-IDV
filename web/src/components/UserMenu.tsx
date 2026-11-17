import { FormEvent, useEffect, useState } from "react";
import { api } from "../api";
import { useAuth } from "../auth";
import { useMessage } from "../message";
import { Modal } from "./Modal";

const ROLE_LABEL: Record<string, string> = {
  super: "超级管理员",
  lab_admin: "机房管理员",
  readonly: "只读",
};

export function UserMenu() {
  const { user, logout, setUser } = useAuth();
  const msg = useMessage();
  const [open, setOpen] = useState(false);
  const [profileOpen, setProfileOpen] = useState(false);
  const [username, setUsername] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [formError, setFormError] = useState("");

  useEffect(() => {
    if (!open) return;
    function close() {
      setOpen(false);
    }
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [open]);

  function openProfile() {
    setOpen(false);
    setUsername(user?.username || "");
    setDisplayName(user?.display_name || "");
    setCurrentPassword("");
    setNewPassword("");
    setConfirmPassword("");
    setFormError("");
    setProfileOpen(true);
  }

  async function onSave(e: FormEvent) {
    e.preventDefault();
    setFormError("");
    if (newPassword && newPassword !== confirmPassword) {
      setFormError("两次输入的新密码不一致");
      return;
    }
    if (newPassword && !currentPassword) {
      setFormError("修改密码需要填写当前密码");
      return;
    }
    try {
      const saved = await api.updateMe({
        username,
        display_name: displayName,
        current_password: currentPassword,
        new_password: newPassword,
      });
      setUser(saved);
      setProfileOpen(false);
      msg.success("已更新个人信息");
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "保存失败");
    }
  }

  const role = ROLE_LABEL[user?.role || ""] || user?.role || "";

  return (
    <>
      <div className="user-menu">
        <button
          type="button"
          className="user-menu-btn"
          aria-expanded={open}
          aria-haspopup="menu"
          onClick={(e) => {
            e.stopPropagation();
            setOpen((v) => !v);
          }}
        >
          <span className="user-menu-name">{user?.display_name || user?.username}</span>
          <span className="user-menu-sub">
            {user?.username}
            {role ? ` · ${role}` : ""}
          </span>
        </button>
        {open ? (
          <div className="user-menu-pop" role="menu" onClick={(e) => e.stopPropagation()}>
            <button type="button" role="menuitem" onClick={openProfile}>
              个人信息
            </button>
            <button type="button" role="menuitem" className="danger" onClick={logout}>
              退出
            </button>
          </div>
        ) : null}
      </div>
      <Modal open={profileOpen} title="个人信息" onClose={() => setProfileOpen(false)}>
        <form onSubmit={onSave}>
          {formError ? <p className="error">{formError}</p> : null}
          <div className="field">
            <label>用户名</label>
            <input value={username} onChange={(e) => setUsername(e.target.value)} required />
          </div>
          <div className="field">
            <label>显示名</label>
            <input value={displayName} onChange={(e) => setDisplayName(e.target.value)} />
          </div>
          <p className="muted">角色：{role || "—"}。角色由超级管理员在管理员页调整。</p>
          <div className="field">
            <label>当前密码</label>
            <input
              type="password"
              value={currentPassword}
              onChange={(e) => setCurrentPassword(e.target.value)}
              autoComplete="current-password"
              placeholder="修改密码时填写"
            />
          </div>
          <div className="field">
            <label>新密码</label>
            <input
              type="password"
              value={newPassword}
              onChange={(e) => setNewPassword(e.target.value)}
              autoComplete="new-password"
              placeholder="留空则不修改"
            />
          </div>
          <div className="field">
            <label>确认新密码</label>
            <input
              type="password"
              value={confirmPassword}
              onChange={(e) => setConfirmPassword(e.target.value)}
              autoComplete="new-password"
            />
          </div>
          <div className="modal-actions">
            <button type="button" className="btn ghost" onClick={() => setProfileOpen(false)}>
              取消
            </button>
            <button className="btn" type="submit">
              保存
            </button>
          </div>
        </form>
      </Modal>
    </>
  );
}
