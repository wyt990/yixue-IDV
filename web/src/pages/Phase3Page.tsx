import { useEffect, useState } from "react";
import { api, getToken } from "../api";
import { useAuth } from "../auth";

export function Phase3Page() {
  const { user } = useAuth();
  const [lock, setLock] = useState({
    enabled: false,
    start_hour_utc: 1,
    end_hour_utc: 10,
  });
  const [packages, setPackages] = useState<
    { id: string; name: string; version_label: string; size_bytes: number }[]
  >([]);
  const [assist, setAssist] = useState<
    { id: string; mac: string; kind: string; state: string; requested_by: string }[]
  >([]);
  const [schedules, setSchedules] = useState<
    {
      id: string;
      name: string;
      kind: string;
      mac_or_tag: string;
      hour_utc: number;
      minute_utc: number;
    }[]
  >([]);
  const [pkgName, setPkgName] = useState("patch-demo");
  const [assistMac, setAssistMac] = useState("02:00:00:00:00:01");
  const [msg, setMsg] = useState("");
  const [err, setErr] = useState("");

  async function refresh() {
    const [l, p, a, s] = await Promise.all([
      api.phase3.getClassLock(),
      api.phase3.listSoftware(),
      api.phase3.listAssist(),
      api.phase3.listSchedules(),
    ]);
    setLock(l);
    setPackages(p);
    setAssist(a);
    setSchedules(s);
  }

  useEffect(() => {
    refresh().catch((e) => setErr(String(e.message || e)));
  }, []);

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div>
        <h1>三期运维</h1>
        <p className="muted">
          软件分发 / 远程协助信令 / 上课只读 / 定时电源。当前角色：
          {user?.role || "—"}
        </p>
      </div>
      {err ? <p className="error">{err}</p> : null}
      {msg ? <p className="muted">{msg}</p> : null}

      <section className="card">
        <h2>上课时段只读锁</h2>
        <p className="muted">
          启用后，UTC {lock.start_hour_utc}:00–{lock.end_hour_utc}:00 禁止新建部署（super
          可 force）。
        </p>
        <label className="muted">
          <input
            type="checkbox"
            checked={lock.enabled}
            onChange={(e) => setLock({ ...lock, enabled: e.target.checked })}
          />{" "}
          启用
        </label>
        <div className="form-row" style={{ marginTop: "0.75rem" }}>
          <button
            className="btn"
            type="button"
            onClick={() =>
              api.phase3
                .setClassLock(lock)
                .then(() => {
                  setMsg("class lock saved");
                  return refresh();
                })
                .catch((e) => setErr(String(e.message || e)))
            }
          >
            保存锁设置
          </button>
          <a
            className="btn ghost"
            href={api.phase3.auditExportUrl}
            onClick={(e) => {
              e.preventDefault();
              const token = getToken();
              fetch(api.phase3.auditExportUrl, {
                headers: token ? { Authorization: `Bearer ${token}` } : {},
              })
                .then(async (r) => {
                  const blob = await r.blob();
                  const url = URL.createObjectURL(blob);
                  const a = document.createElement("a");
                  a.href = url;
                  a.download = "idv-audit.csv";
                  a.click();
                  URL.revokeObjectURL(url);
                })
                .catch((er) => setErr(String(er)));
            }}
          >
            导出审计 CSV
          </a>
        </div>
      </section>

      <section className="card">
        <h2>软件包</h2>
        <div className="form-row">
          <input value={pkgName} onChange={(e) => setPkgName(e.target.value)} />
          <button
            className="btn"
            type="button"
            onClick={() =>
              api.phase3
                .createSoftware(pkgName)
                .then(() => refresh())
                .catch((e) => setErr(String(e.message || e)))
            }
          >
            登记包元数据
          </button>
        </div>
        <ul className="muted">
          {packages.map((p) => (
            <li key={p.id}>
              {p.name} {p.version_label} · {p.size_bytes} B
            </li>
          ))}
        </ul>
      </section>

      <section className="card">
        <h2>远程协助会话</h2>
        <div className="form-row">
          <input value={assistMac} onChange={(e) => setAssistMac(e.target.value)} />
          <button
            className="btn"
            type="button"
            onClick={() =>
              api.phase3
                .createAssist(assistMac, "view")
                .then(() => refresh())
                .catch((e) => setErr(String(e.message || e)))
            }
          >
            发起只读协助
          </button>
        </div>
        <ul className="muted">
          {assist.map((s) => (
            <li key={s.id}>
              {s.mac} · {s.kind} · {s.state} · by {s.requested_by}
            </li>
          ))}
        </ul>
      </section>

      <section className="card">
        <h2>定时电源策略</h2>
        <ul className="muted">
          {schedules.length === 0 ? <li>暂无（可用 API 创建）</li> : null}
          {schedules.map((s) => (
            <li key={s.id}>
              {s.name} · {s.kind} @ {s.hour_utc}:{String(s.minute_utc).padStart(2, "0")} UTC →{" "}
              {s.mac_or_tag}
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
