import { useEffect, useRef, useState, type KeyboardEvent, type MouseEvent, type WheelEvent } from "react";
import { Link } from "react-router-dom";
import { api } from "../api";
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

export type AssistPeer = {
  mac: string;
  name: string;
  online?: boolean;
  last_ip?: string;
};

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export function AssistStage({
  mac,
  mode,
  peers,
  onClose,
  onSwitch,
}: {
  mac: string;
  mode: "view" | "control";
  peers: AssistPeer[];
  onClose: () => void;
  onSwitch: (mac: string) => void;
}) {
  const msg = useMessage();
  const [session, setSession] = useState<Session | null>(null);
  const [frameUrl, setFrameUrl] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [clipboard, setClipboard] = useState("");
  const [tool, setTool] = useState<"clip" | "files">("clip");
  const [path, setPath] = useState("C:\\");
  const [entries, setEntries] = useState<Entry[]>([]);
  const [fsBusy, setFsBusy] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const sessionId = useRef("");
  const frameUrlRef = useRef<string | null>(null);
  const lastMove = useRef(0);
  const assist = mode === "control";
  const peer = peers.find((p) => p.mac === mac);

  useEffect(() => {
    let cancelled = false;
    setSession(null);
    setFrameUrl(null);
    setError("");
    setEntries([]);
    setPath("C:\\");
    (async () => {
      try {
        const s = (await api.phase3.createAssist(mac, mode)) as Session;
        if (cancelled) {
          api.remote.end(s.id).catch(() => {});
          return;
        }
        sessionId.current = s.id;
        setSession({
          ...s,
          clipboard: "",
          detail: s.detail || "",
          frame_count: 0,
        });
      } catch (e) {
        if (!cancelled) setError(String((e as Error).message || e));
      }
    })();
    return () => {
      cancelled = true;
      const id = sessionId.current;
      sessionId.current = "";
      if (id) api.remote.end(id).catch(() => {});
      if (frameUrlRef.current) {
        URL.revokeObjectURL(frameUrlRef.current);
        frameUrlRef.current = null;
      }
    };
  }, [mac, mode, attempt]);

  useEffect(() => {
    if (!session) return;
    const timer = window.setInterval(() => {
      api.remote
        .get(session.id)
        .then((s) => setSession((prev) => (prev ? { ...prev, ...s } : prev)))
        .catch(() => {});
      if (!assist) return;
      api.remote
        .clipboardShow(session.id)
        .then((r) => setClipboard(r.text))
        .catch(() => {});
    }, 2000);
    return () => window.clearInterval(timer);
  }, [session?.id, assist]);

  useEffect(() => {
    if (!session || session.state === "ended" || session.state === "denied") return;
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
        // Session may still be starting.
      }
      if (!stopped) timer = window.setTimeout(tick, 350);
    };
    tick();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, [session?.id, session?.state]);

  async function finish() {
    const id = sessionId.current;
    sessionId.current = "";
    if (id) {
      try {
        await api.remote.end(id);
      } catch {
        // Closing the stage is enough if the server already dropped it.
      }
    }
    onClose();
  }

  function norm(e: MouseEvent<HTMLImageElement>) {
    const rect = e.currentTarget.getBoundingClientRect();
    return {
      x: Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width)),
      y: Math.min(1, Math.max(0, (e.clientY - rect.top) / rect.height)),
    };
  }

  async function sendInput(event: unknown) {
    if (!session || !assist) return;
    try {
      await api.remote.input(session.id, event);
    } catch (e) {
      setError(String((e as Error).message || e));
    }
  }

  function onMouseMove(e: MouseEvent<HTMLImageElement>) {
    if (!assist) return;
    const now = Date.now();
    if (now - lastMove.current < 60) return;
    lastMove.current = now;
    const p = norm(e);
    sendInput({ type: "mouse_move", ...p });
  }

  function onMouseButton(e: MouseEvent<HTMLImageElement>, down: boolean) {
    if (!assist) return;
    e.preventDefault();
    const p = norm(e);
    const button = e.button === 2 ? "right" : e.button === 1 ? "middle" : "left";
    sendInput({ type: "mouse_move", ...p }).then(() =>
      sendInput({ type: "mouse_click", button, down }),
    );
  }

  function onWheel(e: WheelEvent<HTMLImageElement>) {
    if (!assist) return;
    e.preventDefault();
    sendInput({ type: "mouse_wheel", delta: e.deltaY > 0 ? -120 : 120 });
  }

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (!session || !assist) return;
    if (e.key.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey) {
      e.preventDefault();
      sendInput({ type: "text", text: e.key });
      return;
    }
    if (
      e.key.length === 1 ||
      ["Enter", "Backspace", "Tab", "Escape"].includes(e.key) ||
      e.key.startsWith("Arrow")
    ) {
      e.preventDefault();
      sendInput({ type: "key", key: e.key, down: true });
    }
  }

  function onKeyUp(e: KeyboardEvent<HTMLDivElement>) {
    if (!session || !assist) return;
    if (e.key.length > 1) sendInput({ type: "key", key: e.key, down: false });
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

  const parentPath = path.replace(/[\\/][^\\/]*$/, "") || path;

  return (
    <section className={`assist-stage${assist ? "" : " view-only"}`}>
      <div className="assist-top">
        <button type="button" className="btn ghost sm" onClick={finish}>
          返回列表
        </button>
        <strong>{peer?.name || mac}</strong>
        <span className="mono muted">{peer?.last_ip || mac}</span>
        <span className="tag">{assist ? "协助" : "观察"}</span>
        {session ? (
          <span className="muted">
            {session.state}
            {session.frame_count ? ` · ${session.frame_count} 帧` : ""}
            {session.detail ? ` · ${session.detail}` : ""}
          </span>
        ) : (
          <span className="muted">{error ? "未能发起" : "正在发起会话…"}</span>
        )}
        <span className="command-spacer" />
        <Link className="btn ghost sm" to="/tasks">
          查看任务
        </Link>
        <button type="button" className="btn sm" onClick={finish}>
          结束会话
        </button>
      </div>
      {error ? <p className="error">{error}</p> : null}
      {!session && error ? (
        <button type="button" className="btn sm" onClick={() => setAttempt((n) => n + 1)}>
          重试
        </button>
      ) : null}
      <div className="assist-main">
        <div
          className="assist-screen"
          tabIndex={0}
          onKeyDown={onKeyDown}
          onKeyUp={onKeyUp}
        >
          {frameUrl ? (
            <img
              src={frameUrl}
              alt="终端画面"
              draggable={false}
              onMouseMove={onMouseMove}
              onMouseDown={(e) => onMouseButton(e, true)}
              onMouseUp={(e) => onMouseButton(e, false)}
              onWheel={onWheel}
              onContextMenu={(e) => e.preventDefault()}
              style={{ cursor: assist ? "crosshair" : "default" }}
            />
          ) : (
            <p className="muted">
              {session ? "等待客户端首帧画面…" : "会话准备中"}
            </p>
          )}
        </div>
        {assist ? (
          <aside className="assist-rail">
            <div className="assist-tabs">
              <button
                type="button"
                className={tool === "clip" ? "active" : ""}
                onClick={() => setTool("clip")}
              >
                剪贴板
              </button>
              <button
                type="button"
                className={tool === "files" ? "active" : ""}
                onClick={() => setTool("files")}
              >
                文件
              </button>
            </div>
            {tool === "clip" ? (
              <div className="assist-pane">
                <textarea
                  value={clipboard}
                  onChange={(e) => setClipboard(e.target.value)}
                  rows={8}
                  placeholder="协助模式下的文本剪贴板"
                />
                <div className="page-actions">
                  <button
                    className="btn ghost sm"
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
                    className="btn sm"
                    type="button"
                    disabled={!session}
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
            ) : (
              <div className="assist-pane">
                <div className="page-actions">
                  <input value={path} onChange={(e) => setPath(e.target.value)} />
                  <button
                    className="btn sm"
                    type="button"
                    disabled={!session || fsBusy}
                    onClick={() => openPath(path)}
                  >
                    打开
                  </button>
                  <button
                    className="btn ghost sm"
                    type="button"
                    disabled={!session || fsBusy}
                    onClick={() => openPath(parentPath)}
                  >
                    上级
                  </button>
                  <label className="btn ghost sm">
                    上传
                    <input
                      type="file"
                      hidden
                      onChange={(e) => {
                        const f = e.target.files?.[0];
                        if (f) uploadFile(f);
                        e.currentTarget.value = "";
                      }}
                    />
                  </label>
                </div>
                {fsBusy ? <p className="muted">文件操作中…</p> : null}
                <div className="assist-files">
                  <table className="data-table">
                    <tbody>
                      {entries.length === 0 ? (
                        <tr>
                          <td className="muted">{session ? "打开目录后显示" : "等待会话"}</td>
                        </tr>
                      ) : (
                        entries.map((entry) => (
                          <tr key={entry.path}>
                            <td>
                              {entry.is_dir ? (
                                <button
                                  type="button"
                                  className="linkish"
                                  onClick={() => openPath(entry.path)}
                                >
                                  {entry.name}
                                </button>
                              ) : (
                                entry.name
                              )}
                            </td>
                            <td className="muted">
                              {entry.is_dir ? "目录" : `${Math.round(entry.size / 1024)} KB`}
                            </td>
                            <td>
                              {!entry.is_dir ? (
                                <button
                                  className="btn ghost sm"
                                  type="button"
                                  onClick={() => downloadEntry(entry)}
                                >
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
            )}
          </aside>
        ) : null}
      </div>
      <div className="assist-peers" aria-label="换一台">
        {peers.map((p) => (
          <button
            key={p.mac}
            type="button"
            className={`assist-peer${p.mac === mac ? " active" : ""}`}
            onClick={() => {
              if (p.mac !== mac) onSwitch(p.mac);
            }}
          >
            <span className={`status-dot${p.online ? " on" : " off"}`} />
            <span>{p.name || p.mac}</span>
            <span className="mono muted">{p.last_ip || ""}</span>
          </button>
        ))}
      </div>
    </section>
  );
}
