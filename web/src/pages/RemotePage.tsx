import { useEffect, useMemo, useRef, useState } from "react";
import { api, type Client, type Lab, type TaskView } from "../api";
import { useMessage } from "../message";

type Session = {
  id: string;
  mac: string;
  kind: string;
  state: string;
  detail: string;
  clipboard: string;
  frame_count: number;
};

type Entry = {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: number;
};

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export function RemotePage() {
  const msg = useMessage();
  const [clients, setClients] = useState<Client[]>([]);
  const [labs, setLabs] = useState<Lab[]>([]);
  const [mac, setMac] = useState("");
  const [mode, setMode] = useState<"view" | "control">("view");
  const [session, setSession] = useState<Session | null>(null);
  const [frameUrl, setFrameUrl] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [clipboard, setClipboard] = useState("");
  const [busy, setBusy] = useState("");
  const frameUrlRef = useRef<string | null>(null);
  const lastMove = useRef(0);

  const [path, setPath] = useState("C:\\");
  const [entries, setEntries] = useState<Entry[]>([]);
  const [fsBusy, setFsBusy] = useState(false);

  const [batchLab, setBatchLab] = useState("");
  const [batchSelected, setBatchSelected] = useState<Set<string>>(new Set());
  const [collectDir, setCollectDir] = useState("C:\\Users\\Public\\Documents");
  const [targetDir, setTargetDir] = useState("C:\\Users\\Public\\Documents");
  const [batchFile, setBatchFile] = useState<File | null>(null);
  const [batchTask, setBatchTask] = useState("");
  const [fileTasks, setFileTasks] = useState<TaskView[]>([]);

  const labMap = useMemo(() => new Map(labs.map((l) => [l.id, l])), [labs]);
  const filteredClients = useMemo(
    () => clients.filter((c) => !batchLab || c.lab_id === batchLab),
    [clients, batchLab],
  );

  async function refresh() {
    const [c, l, t] = await Promise.all([api.clients.list(), api.labs.list(), api.tasks.list()]);
    setClients(c);
    setLabs(l);
    setFileTasks(t.filter((x) => x.kind === "file_collect" || x.kind === "file_distribute"));
    if (!mac && c[0]) setMac(c[0].mac);
    if (!batchLab && l[0]) setBatchLab(l[0].id);
  }

  useEffect(() => {
    refresh().catch((e) => setError(String(e.message || e)));
  }, []);

  useEffect(() => {
    const timer = window.setInterval(() => {
      api.tasks
        .list()
        .then((t) =>
          setFileTasks(
            t.filter((x) => x.kind === "file_collect" || x.kind === "file_distribute"),
          ),
        )
        .catch(() => {});
    }, 5000);
    return () => window.clearInterval(timer);
  }, []);

  // Session status + clipboard poll.
  useEffect(() => {
    if (!session) return;
    const timer = window.setInterval(() => {
      api.remote
        .get(session.id)
        .then((s) => setSession((prev) => (prev ? { ...prev, ...s } : prev)))
        .catch(() => {});
      api.remote
        .clipboardShow(session.id)
        .then((r) => setClipboard(r.text))
        .catch(() => {});
    }, 2000);
    return () => window.clearInterval(timer);
  }, [session?.id]);

  // Screen frame poll (~3 fps).
  useEffect(() => {
    if (!session || session.state === "ended" || session.state === "denied") {
      return;
    }
    let stopped = false;
    let timer = 0;
    const tick = async () => {
      if (stopped) return;
      try {
        const blob = await api.remote.frame(session.id);
        if (blob && !stopped) {
          const url = URL.createObjectURL(blob);
          if (frameUrlRef.current) URL.revokeObjectURL(frameUrlRef.current);
          frameUrlRef.current = url;
          setFrameUrl(url);
        }
      } catch {
        // Session may still be starting; retry.
      }
      if (!stopped) timer = window.setTimeout(tick, 350);
    };
    tick();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
      if (frameUrlRef.current) {
        URL.revokeObjectURL(frameUrlRef.current);
        frameUrlRef.current = null;
      }
    };
  }, [session?.id, session?.state]);

  async function startSession() {
    if (!mac) {
      setError("请选择客户端");
      return;
    }
    setError("");
    setBusy("start");
    try {
      const s = (await api.phase3.createAssist(mac, mode)) as Session;
      setSession({ ...s, clipboard: "", detail: s.detail || "", frame_count: 0 });
      setPath("C:\\");
      setEntries([]);
      msg.success(`已发起${mode === "control" ? "协助" : "观察"}会话`);
    } catch (e) {
      setError(String((e as Error).message || e));
    } finally {
      setBusy("");
    }
  }

  async function endSession() {
    if (!session) return;
    try {
      await api.remote.end(session.id);
    } catch {
      // ignore
    }
    setSession(null);
    setFrameUrl(null);
  }

  function norm(e: React.MouseEvent<HTMLImageElement>) {
    const rect = e.currentTarget.getBoundingClientRect();
    return {
      x: Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width)),
      y: Math.min(1, Math.max(0, (e.clientY - rect.top) / rect.height)),
    };
  }

  async function sendInput(event: unknown) {
    if (!session || session.kind !== "control") return;
    try {
      await api.remote.input(session.id, event);
    } catch (e) {
      setError(String((e as Error).message || e));
    }
  }

  function onMouseMove(e: React.MouseEvent<HTMLImageElement>) {
    const now = Date.now();
    if (now - lastMove.current < 60) return;
    lastMove.current = now;
    const p = norm(e);
    sendInput({ type: "mouse_move", ...p });
  }

  function onMouseButton(e: React.MouseEvent<HTMLImageElement>, down: boolean) {
    e.preventDefault();
    const p = norm(e);
    const button = e.button === 2 ? "right" : e.button === 1 ? "middle" : "left";
    sendInput({ type: "mouse_move", ...p }).then(() =>
      sendInput({ type: "mouse_click", button, down }),
    );
  }

  function onWheel(e: React.WheelEvent<HTMLImageElement>) {
    e.preventDefault();
    sendInput({ type: "mouse_wheel", delta: e.deltaY > 0 ? -120 : 120 });
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    if (!session || session.kind !== "control") return;
    if (e.key.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey) {
      e.preventDefault();
      sendInput({ type: "text", text: e.key });
      return;
    }
    if (e.key.length === 1 || ["Enter", "Backspace", "Tab", "Escape"].includes(e.key) || e.key.startsWith("Arrow")) {
      e.preventDefault();
      sendInput({ type: "key", key: e.key, down: true });
    }
  }

  function onKeyUp(e: React.KeyboardEvent<HTMLDivElement>) {
    if (!session || session.kind !== "control") return;
    if (e.key.length > 1) {
      sendInput({ type: "key", key: e.key, down: false });
    }
  }

  async function waitFs(requestId: string, timeoutMs = 60000) {
    const started = Date.now();
    while (Date.now() - started < timeoutMs) {
      const st = await api.remote.fsStatus(requestId);
      if (st.state === "done" || st.state === "failed") return st;
      await sleep(400);
    }
    throw new Error("文件操作超时");
  }

  async function openPath(next: string) {
    if (!session) return;
    setFsBusy(true);
    setError("");
    try {
      const { request_id } = await api.remote.fsList(session.id, next);
      const st = await waitFs(request_id);
      if (st.state === "failed") throw new Error(st.error_message || "列目录失败");
      const parsed = st.payload ? JSON.parse(st.payload) : { entries: [] };
      setEntries((parsed.entries || []) as Entry[]);
      setPath(next);
    } catch (e) {
      setError(String((e as Error).message || e));
    } finally {
      setFsBusy(false);
    }
  }

  async function downloadEntry(entry: Entry) {
    if (!session) return;
    setFsBusy(true);
    try {
      const { request_id } = await api.remote.fsDownload(session.id, entry.path);
      const st = await waitFs(request_id);
      if (st.state === "failed") throw new Error(st.error_message || "下载失败");
      await api.remote.fsData(request_id, entry.name);
      msg.success(`已下载 ${entry.name}`);
    } catch (e) {
      setError(String((e as Error).message || e));
    } finally {
      setFsBusy(false);
    }
  }

  async function uploadFile(file: File) {
    if (!session) return;
    setFsBusy(true);
    try {
      const target = path.endsWith("\\") ? `${path}${file.name}` : `${path}\\${file.name}`;
      const { request_id } = await api.remote.fsUpload(session.id, target, file);
      const st = await waitFs(request_id);
      if (st.state === "failed") throw new Error(st.error_message || "上传失败");
      msg.success(`已上传 ${file.name}`);
      await openPath(path);
    } catch (e) {
      setError(String((e as Error).message || e));
    } finally {
      setFsBusy(false);
    }
  }

  async function doCollect() {
    if (!batchSelected.size) {
      setError("请先勾选目标客户端");
      return;
    }
    try {
      const r = await api.remote.fileCollect({
        macs: [...batchSelected],
        source_dir: collectDir,
      });
      setBatchTask(`收集任务 ${r.task_id}（${r.targets} 台），到“维护任务”查看进度`);
      msg.success("已下发文件收集任务");
    } catch (e) {
      setError(String((e as Error).message || e));
    }
  }

  async function doDistribute() {
    if (!batchFile) {
      setError("请选择要下发的 zip 文件");
      return;
    }
    if (!batchSelected.size) {
      setError("请先勾选目标客户端");
      return;
    }
    try {
      const r = await api.remote.fileDistribute(targetDir, [...batchSelected], batchFile);
      setBatchTask(`下发任务 ${r.task_id}（${r.targets} 台），到“维护任务”查看进度`);
      msg.success("已下发文件分发任务");
    } catch (e) {
      setError(String((e as Error).message || e));
    }
  }

  function toggleBatchMac(target: string, on: boolean) {
    setBatchSelected((prev) => {
      const next = new Set(prev);
      if (on) next.add(target);
      else next.delete(target);
      return next;
    });
  }

  const parentPath = path.replace(/[\\/][^\\/]*$/, "") || path;

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div className="panel-head">
        <div>
          <h1>远程控制</h1>
          <p className="muted">
            观察模式只看屏幕；协助模式可键鼠控制、共享剪贴板、互传文件；下方支持批量收集/下发。
          </p>
        </div>
      </div>
      {error ? <p className="error">{error}</p> : null}

      <div className="card">
        <div className="page-actions" style={{ flexWrap: "wrap", alignItems: "center" }}>
          <select value={mac} onChange={(e) => setMac(e.target.value)}>
            {clients.map((c) => (
              <option key={c.mac} value={c.mac}>
                {c.name || c.mac} · {c.mac}
                {c.agent_version ? ` · v${c.agent_version}` : ""}
              </option>
            ))}
          </select>
          <select value={mode} onChange={(e) => setMode(e.target.value as "view" | "control")}>
            <option value="view">观察模式</option>
            <option value="control">协助模式</option>
          </select>
          <button className="btn accent" type="button" onClick={startSession} disabled={!!busy}>
            发起会话
          </button>
          {session ? (
            <>
              <span className="tag">
                {session.kind === "control" ? "协助" : "观察"} · {session.state} · {session.frame_count} 帧
              </span>
              {session.detail ? (
                <span className="muted">{session.detail}</span>
              ) : null}
              <button className="btn ghost" type="button" onClick={endSession}>
                结束
              </button>
            </>
          ) : null}
        </div>
        <div
          tabIndex={0}
          onKeyDown={onKeyDown}
          onKeyUp={onKeyUp}
          style={{ outline: "none", marginTop: "0.75rem" }}
        >
          {frameUrl ? (
            <img
              src={frameUrl}
              alt="remote screen"
              draggable={false}
              onMouseMove={onMouseMove}
              onMouseDown={(e) => onMouseButton(e, true)}
              onMouseUp={(e) => onMouseButton(e, false)}
              onWheel={onWheel}
              onContextMenu={(e) => e.preventDefault()}
              style={{
                width: "100%",
                maxHeight: "70vh",
                objectFit: "contain",
                background: "#000",
                cursor: session?.kind === "control" ? "crosshair" : "default",
              }}
            />
          ) : (
            <div className="muted" style={{ padding: "3rem", textAlign: "center" }}>
              {session ? "等待客户端首帧画面…（需客户端已登录用户会话运行）" : "选择客户端并发起会话"}
            </div>
          )}
        </div>
      </div>

      <div className="card">
        <h2>剪贴板</h2>
        <div className="form-row" style={{ alignItems: "flex-start" }}>
          <textarea
            value={clipboard}
            onChange={(e) => setClipboard(e.target.value)}
            rows={3}
            style={{ flex: 1 }}
            placeholder="客户端/本机剪贴板文本"
          />
          <div className="page-actions" style={{ flexDirection: "column" }}>
            <button
              className="btn ghost"
              type="button"
              disabled={!session}
              onClick={() =>
                session &&
                api.remote
                  .clipboardRead(session.id)
                  .then(() => msg.success("已请求读取客户端剪贴板"))
                  .catch((e) => setError(String(e.message || e)))
              }
            >
              读取客户端
            </button>
            <button
              className="btn"
              type="button"
              disabled={!session || session.kind !== "control"}
              onClick={() =>
                session &&
                api.remote
                  .clipboardSet(session.id, clipboard)
                  .then(() => msg.success("已发送到客户端"))
                  .catch((e) => setError(String(e.message || e)))
              }
            >
              发送到客户端
            </button>
          </div>
        </div>
      </div>

      <div className="card">
        <h2>文件传输（一对一）</h2>
        <div className="page-actions" style={{ flexWrap: "wrap" }}>
          <input value={path} onChange={(e) => setPath(e.target.value)} style={{ flex: 1 }} />
          <button className="btn" type="button" disabled={!session || fsBusy} onClick={() => openPath(path)}>
            打开
          </button>
          <button className="btn ghost" type="button" disabled={!session || fsBusy} onClick={() => openPath(parentPath)}>
            上级
          </button>
          <label className="btn ghost" style={{ cursor: "pointer" }}>
            上传
            <input
              type="file"
              style={{ display: "none" }}
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f) uploadFile(f);
                e.currentTarget.value = "";
              }}
            />
          </label>
        </div>
        {fsBusy ? <p className="muted">文件操作中…</p> : null}
        <div style={{ maxHeight: "18rem", overflow: "auto" }}>
          <table>
            <thead>
              <tr>
                <th>名称</th>
                <th>大小</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              {entries.length === 0 ? (
                <tr>
                  <td className="muted" colSpan={3}>
                    {session ? "空目录" : "先发起会话"}
                  </td>
                </tr>
              ) : (
                entries.map((entry) => (
                  <tr key={entry.path}>
                    <td>
                      {entry.is_dir ? (
                        <button
                          className="link"
                          type="button"
                          onClick={() => openPath(entry.path)}
                          style={{ background: "none", border: 0, color: "inherit", cursor: "pointer" }}
                        >
                          📁 {entry.name}
                        </button>
                      ) : (
                        <span>📄 {entry.name}</span>
                      )}
                    </td>
                    <td className="muted">{entry.is_dir ? "—" : `${Math.round(entry.size / 1024)} KB`}</td>
                    <td>
                      {!entry.is_dir ? (
                        <button className="btn ghost" type="button" onClick={() => downloadEntry(entry)}>
                          下载
                        </button>
                      ) : null}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </div>

      <div className="card">
        <h2>批量文件收集 / 下发</h2>
        <p className="muted">
          收集：客户端把指定目录打成 zip 上传；下发：上传 zip，客户端解压到指定目录。结果在“维护任务”查看。
        </p>
        {batchTask ? <p className="muted">{batchTask}</p> : null}
        <div className="page-actions" style={{ flexWrap: "wrap" }}>
          <select value={batchLab} onChange={(e) => setBatchLab(e.target.value)}>
            {labs.map((l) => (
              <option key={l.id} value={l.id}>
                {l.name}
              </option>
            ))}
          </select>
          <button
            className="btn ghost"
            type="button"
            onClick={() =>
              setBatchSelected((prev) => {
                const next = new Set(prev);
                filteredClients.forEach((c) => next.add(c.mac));
                return next;
              })
            }
          >
            全选本机房
          </button>
          <button className="btn ghost" type="button" onClick={() => setBatchSelected(new Set())}>
            清除
          </button>
          <span className="muted">已选 {batchSelected.size} 台</span>
        </div>
        <div style={{ maxHeight: "10rem", overflow: "auto", marginTop: "0.5rem" }}>
          <table>
            <tbody>
              {filteredClients.map((c) => (
                <tr key={c.mac}>
                  <td style={{ width: "2rem" }}>
                    <input
                      type="checkbox"
                      checked={batchSelected.has(c.mac)}
                      onChange={(e) => toggleBatchMac(c.mac, e.target.checked)}
                    />
                  </td>
                  <td>{c.name || "—"}</td>
                  <td className="mono">{c.mac}</td>
                  <td className="muted">{labMap.get(c.lab_id)?.name}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="form-row" style={{ marginTop: "0.75rem", flexWrap: "wrap" }}>
          <input
            value={collectDir}
            onChange={(e) => setCollectDir(e.target.value)}
            placeholder="收集目录，如 C:\Users\Public\Documents"
            style={{ flex: 1 }}
          />
          <button className="btn" type="button" onClick={doCollect}>
            收集到服务器
          </button>
        </div>
        <div className="form-row" style={{ marginTop: "0.75rem", flexWrap: "wrap" }}>
          <input
            value={targetDir}
            onChange={(e) => setTargetDir(e.target.value)}
            placeholder="下发目标目录，如 C:\Users\Public\Documents"
            style={{ flex: 1 }}
          />
          <input type="file" accept=".zip" onChange={(e) => setBatchFile(e.target.files?.[0] ?? null)} />
          <button className="btn" type="button" onClick={doDistribute}>
            下发到客户端
          </button>
        </div>
      </div>

      <div className="card">
        <h2>最近文件任务</h2>
        <p className="muted">收集成功的客户端可下载 zip；下发任务可在“维护任务”查看失败原因。</p>
        <table>
          <thead>
            <tr>
              <th>任务</th>
              <th>目标</th>
              <th>状态</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {fileTasks.length === 0 ? (
              <tr>
                <td className="muted" colSpan={4}>
                  暂无文件任务
                </td>
              </tr>
            ) : (
              fileTasks.slice(0, 8).flatMap((task) =>
                task.targets.map((target) => (
                  <tr key={`${task.id}-${target.mac}`}>
                    <td>
                      {task.kind === "file_collect" ? "收集" : "下发"} · {task.name}
                    </td>
                    <td className="mono">{target.mac}</td>
                    <td className="muted">{target.state}</td>
                    <td>
                      {task.kind === "file_collect" && target.state === "done" ? (
                        <button
                          className="btn ghost"
                          type="button"
                          onClick={() =>
                            api.remote
                              .collectArchive(task.id, target.mac)
                              .catch((e) => setError(String(e.message || e)))
                          }
                        >
                          下载 zip
                        </button>
                      ) : (
                        "—"
                      )}
                    </td>
                  </tr>
                )),
              )
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
