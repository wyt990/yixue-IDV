import { FormEvent, useState } from "react";
import { Navigate } from "react-router-dom";
import { useAuth } from "../auth";
import { useTheme, type ThemeId } from "../theme";

export function LoginPage() {
  const { user, loading, login } = useAuth();
  const { theme, setTheme } = useTheme();
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("admin123");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  if (!loading && user) return <Navigate to="/" replace />;

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      await login(username, password);
    } catch (err) {
      setError(err instanceof Error ? err.message : "登录失败");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="login-page">
      <div className="card login-card">
        <div className="brand-title" style={{ color: "var(--brand)" }}>
          易学 IDV
        </div>
        <p className="muted" style={{ marginTop: "0.35rem" }}>
          机房管理系统 · 管理端登录
        </p>
        <form onSubmit={onSubmit} style={{ marginTop: "1.25rem" }}>
          <div className="field">
            <label htmlFor="username">账号</label>
            <input
              id="username"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              autoComplete="username"
              required
            />
          </div>
          <div className="field">
            <label htmlFor="password">密码</label>
            <input
              id="password"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
              required
            />
          </div>
          {error ? <p className="error">{error}</p> : null}
          <button className="btn" type="submit" disabled={busy} style={{ width: "100%" }}>
            {busy ? "登录中…" : "登录"}
          </button>
        </form>
        <div className="theme-dots" aria-label="主题预览">
          {(
            [
              ["ink-lab", "#1B3A4B"],
              ["pine-rack", "#1F6F5B"],
              ["slate-ops", "#3D4A54"],
              ["night-duty", "#12161C"],
            ] as [ThemeId, string][]
          ).map(([id, color]) => (
            <button
              key={id}
              type="button"
              className={`theme-dot${theme === id ? " active" : ""}`}
              style={{ background: color }}
              aria-label={id}
              onClick={() => setTheme(id)}
            />
          ))}
        </div>
        <p className="muted" style={{ fontSize: "0.82rem", marginTop: "0.85rem" }}>
          默认账号 admin / admin123
        </p>
      </div>
    </div>
  );
}
