import { useTheme, type ThemeId } from "../theme";

const themes: { id: ThemeId; name: string; desc: string }[] = [
  { id: "ink-lab", name: "墨蓝教研", desc: "日间网管默认，深墨蓝 + 亮青信息色" },
  { id: "pine-rack", name: "青松机柜", desc: "机房/网络气质，松绿 + 石墨结构" },
  { id: "slate-ops", name: "暖石运维", desc: "报表阅读向，石板灰 + 琥珀强调" },
  { id: "night-duty", name: "暗夜值班", desc: "夜间盯任务，炭底 + 月白字" },
];

export function AppearancePage() {
  const { theme, setTheme } = useTheme();

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div>
        <h1>外观主题</h1>
        <p className="muted">
          基于 Design Tokens 切换；偏好保存在本机。信创浏览器抽检见文档「模块
          13 — 信创浏览器兼容矩阵」。
        </p>
      </div>
      <div className="form-row">
        {themes.map((t) => (
          <button
            key={t.id}
            type="button"
            className="card"
            style={{
              textAlign: "left",
              cursor: "pointer",
              borderColor: theme === t.id ? "var(--accent)" : undefined,
            }}
            onClick={() => setTheme(t.id)}
          >
            <h2>{t.name}</h2>
            <p className="muted">{t.desc}</p>
            {theme === t.id ? <span className="tag">当前</span> : null}
          </button>
        ))}
      </div>
    </div>
  );
}
