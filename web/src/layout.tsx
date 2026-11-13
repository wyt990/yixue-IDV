import { NavLink, Outlet } from "react-router-dom";
import { useAuth } from "./auth";
import { useTheme, type ThemeId } from "./theme";

const links = [
  { to: "/", label: "工作台", end: true },
  { to: "/labs", label: "校区 / 机房" },
  { to: "/clients", label: "终端" },
  { to: "/images", label: "镜像库" },
  { to: "/edges", label: "边缘节点" },
  { to: "/pe-sessions", label: "PE 会话" },
  { to: "/tasks", label: "维护任务" },
  { to: "/modes", label: "工作模式" },
  { to: "/phase3", label: "三期运维" },
  { to: "/admins", label: "管理员" },
  { to: "/appearance", label: "外观主题" },
];

export function AppLayout() {
  const { user, logout } = useAuth();
  const { theme, setTheme } = useTheme();

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark" aria-hidden>
            IDV
          </div>
          <div className="brand-title">易学 IDV</div>
          <div className="brand-sub">机房管理系统</div>
        </div>
        <nav className="nav">
          {links.map((link) => (
            <NavLink
              key={link.to}
              to={link.to}
              end={link.end}
              className={({ isActive }) => (isActive ? "active" : undefined)}
            >
              {link.label}
            </NavLink>
          ))}
        </nav>
      </aside>
      <div className="main">
        <header className="topbar">
          <div className="muted">本地运算 · 集中管控</div>
          <div style={{ display: "flex", gap: "0.75rem", alignItems: "center" }}>
            <select
              aria-label="主题"
              value={theme}
              onChange={(e) => setTheme(e.target.value as ThemeId)}
            >
              <option value="ink-lab">墨蓝教研</option>
              <option value="pine-rack">青松机柜</option>
              <option value="slate-ops">暖石运维</option>
              <option value="night-duty">暗夜值班</option>
            </select>
            <span className="muted">
              {user?.display_name}
              {user?.role ? ` · ${user.role}` : ""}
            </span>
            <button className="btn ghost" type="button" onClick={logout}>
              退出
            </button>
          </div>
        </header>
        <main className="content">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
