import { FormEvent, useEffect, useState } from "react";
import { api, type Lab, type ModePolicy } from "../api";

export function ModesPage() {
  const [modes, setModes] = useState<ModePolicy[]>([]);
  const [labs, setLabs] = useState<Lab[]>([]);
  const [bindings, setBindings] = useState<
    { lab_id: string; mode_policy_id: string; effective: string }[]
  >([]);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [labId, setLabId] = useState("");
  const [modeId, setModeId] = useState("");

  async function refresh() {
    const [m, l, b] = await Promise.all([
      api.modes.list(),
      api.labs.list(),
      api.modes.bindings(),
    ]);
    setModes(m);
    setLabs(l);
    setBindings(b);
    if (!labId && l[0]) setLabId(l[0].id);
    if (!modeId && m[0]) setModeId(m[0].id);
  }

  useEffect(() => {
    refresh().catch((err) => setError(err.message));
  }, []);

  async function bind(e: FormEvent) {
    e.preventDefault();
    setError("");
    try {
      await api.modes.bindLab(labId, modeId, "next_boot");
      setMessage("已绑定机房工作模式");
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "绑定失败");
    }
  }

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div>
        <h1>工作模式</h1>
        <p className="muted">
          教学还原 / 办公 / 考试：分区角色策略下发给 idv-agent。
        </p>
      </div>
      {error ? <p className="error">{error}</p> : null}
      {message ? <p className="muted">{message}</p> : null}

      <div className="card">
        <h2>模式策略</h2>
        <table>
          <thead>
            <tr>
              <th>名称</th>
              <th>restore</th>
              <th>system</th>
              <th>persistent</th>
              <th>volatile</th>
              <th>时间窗 / USB</th>
            </tr>
          </thead>
          <tbody>
            {modes.map((m) => (
              <tr key={m.id}>
                <td>
                  {m.name}
                  {m.notes ? (
                    <div className="muted" style={{ fontSize: "0.85rem" }}>
                      {m.notes}
                    </div>
                  ) : null}
                </td>
                <td className="mono">{m.restore}</td>
                <td className="mono">{m.system_action}</td>
                <td className="mono">{m.persistent_action}</td>
                <td className="mono">{m.volatile_action}</td>
                <td className="muted" style={{ fontSize: "0.85rem" }}>
                  {m.schedule_start || m.schedule_end
                    ? `${m.schedule_start || "—"} → ${m.schedule_end || "—"}`
                    : "全时段"}
                  {m.usb_restricted ? " · USB限制" : ""}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <form className="card" onSubmit={bind}>
        <h2>绑定机房</h2>
        <div className="form-row">
          <div className="field">
            <label>机房</label>
            <select value={labId} onChange={(e) => setLabId(e.target.value)}>
              {labs.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.name}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>模式</label>
            <select value={modeId} onChange={(e) => setModeId(e.target.value)}>
              {modes.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                </option>
              ))}
            </select>
          </div>
        </div>
        <button className="btn" type="submit" disabled={!labId || !modeId}>
          绑定（下开机生效）
        </button>
      </form>

      <div className="card">
        <h2>当前绑定</h2>
        <table>
          <thead>
            <tr>
              <th>机房</th>
              <th>模式</th>
              <th>生效</th>
            </tr>
          </thead>
          <tbody>
            {bindings.map((b) => (
              <tr key={b.lab_id}>
                <td className="mono">
                  {labs.find((l) => l.id === b.lab_id)?.name || b.lab_id}
                </td>
                <td>
                  {modes.find((m) => m.id === b.mode_policy_id)?.name ||
                    b.mode_policy_id}
                </td>
                <td className="mono">{b.effective}</td>
              </tr>
            ))}
            {bindings.length === 0 ? (
              <tr>
                <td colSpan={3} className="muted">
                  暂无绑定
                </td>
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>
    </div>
  );
}
