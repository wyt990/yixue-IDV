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
  const [error, setError] = useState("");
  const [clipboard, setClipboard] = useState("");
  const [tool, setTool] = useState<"clip" | "files">("clip");
  const [path, setPath] = useState("C:\\");
  const [entries, setEntries] = useState<Entry[]>([]);
  const [fsBusy, setFsBusy] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [streamStats, setStreamStats] = useState({ fps: 0, rtt: 0, codec: "JPEG" });
  const sessionId = useRef("");
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const pendingMove = useRef<{ x: number; y: number } | null>(null);
  const moveTimer = useRef<number | undefined>(undefined);
  const lastRtt = useRef(0);
  const frameCounter = useRef({ at: Date.now(), frames: 0 });
  const h264Decoder = useRef<VideoDecoder | null>(null);
  const h264Seq = useRef(0);
  const downloads = useRef(
    new Map<string, { name: string; chunks: Uint8Array[] }>(),
  );
  const assist = mode === "control";
  const peer = peers.find((p) => p.mac === mac);

  useEffect(() => {
    let cancelled = false;
    setSession(null);
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
    };
  }, [mac, mode, attempt]);

  useEffect(() => {
    if (!session) return;
    const timer = window.setInterval(() => {
      api.remote
        .get(session.id)
        .then((s) => setSession((prev) => (prev ? { ...prev, ...s } : prev)))
        .catch(() => {});
      api.remote
        .get(session.id)
        .then((s) => setSession((prev) => (prev ? { ...prev, ...s } : prev)))
        .catch(() => {});
    }, 2000);
    return () => window.clearInterval(timer);
  }, [session?.id, assist]);

  useEffect(() => {
    if (!session || session.state === "ended" || session.state === "denied") return;
    const ws = new WebSocket(api.remote.wsUrl(session.id));
    ws.binaryType = "arraybuffer";
    wsRef.current = ws;
    ws.onmessage = async (ev) => {
      if (typeof ev.data === "string") {
        try {
          const m = JSON.parse(ev.data) as {
            type?: string;
            text?: string;
            id?: string;
            name?: string;
            data?: string;
            message?: string;
            codec?: string;
            description?: string;
            width?: number;
            height?: number;
          };
          if (m.type === "clipboard" && typeof m.text === "string") setClipboard(m.text);
          if (
            m.type === "h264_config" &&
            typeof m.codec === "string" &&
            typeof m.description === "string"
          ) {
            const bytes = new Uint8Array(m.description.length / 2);
            for (let i = 0; i < bytes.length; i++) {
              bytes[i] = parseInt(m.description.slice(i * 2, i * 2 + 2), 16);
            }
            h264Decoder.current?.close();
            try {
              const decoder = new VideoDecoder({
                output: (frame) => {
                  const canvas = canvasRef.current;
                  if (canvas) {
                    if (m.width && m.height && (canvas.width !== m.width || canvas.height !== m.height)) {
                      canvas.width = m.width;
                      canvas.height = m.height;
                    }
                    canvas.getContext("2d")?.drawImage(frame, 0, 0);
                  }
                  frame.close();
                  h264Seq.current += 1;
                  frameCounter.current.frames += 1;
                  if (wsRef.current?.readyState === WebSocket.OPEN) {
                    wsRef.current.send(
                      JSON.stringify({
                        type: "ack",
                        seq: h264Seq.current,
                        rtt: lastRtt.current,
                      }),
                    );
                  }
                  const now = Date.now();
                  if (now - frameCounter.current.at >= 1000) {
                    const fps = Math.round(
                      (frameCounter.current.frames * 1000) / (now - frameCounter.current.at),
                    );
                    frameCounter.current = { at: now, frames: 0 };
                    setStreamStats((s) => ({ ...s, fps }));
                  }
                },
                error: () => {
                  h264Decoder.current = null;
                  if (wsRef.current?.readyState === WebSocket.OPEN) {
                    wsRef.current.send(JSON.stringify({ type: "codec", codec: "jpeg" }));
                  }
                },
              });
              decoder.configure({
                codec: m.codec,
                description: bytes,
                optimizeForLatency: true,
              });
              h264Decoder.current = decoder;
              setStreamStats((s) => ({ ...s, codec: "H.264" }));
            } catch {
              h264Decoder.current = null;
              setStreamStats((s) => ({ ...s, codec: "JPEG" }));
              if (wsRef.current?.readyState === WebSocket.OPEN) {
                wsRef.current.send(JSON.stringify({ type: "codec", codec: "jpeg" }));
              }
            }
          }
          if (m.type === "pong" && typeof (m as { t?: number }).t === "number") {
            const t = (m as { t: number }).t;
            lastRtt.current = Math.max(0, Math.round(performance.now() - t));
            setStreamStats((s) => ({ ...s, rtt: lastRtt.current }));
          }
          if (m.type === "file_begin" && m.id) {
            downloads.current.set(m.id, { name: m.name || "download", chunks: [] });
          }
          if (m.type === "file_chunk" && m.id && m.data) {
            const item = downloads.current.get(m.id);
            if (item) {
              const bytes = new Uint8Array(m.data.length / 2);
              for (let i = 0; i < bytes.length; i++) {
                bytes[i] = parseInt(m.data.slice(i * 2, i * 2 + 2), 16);
              }
              item.chunks.push(bytes);
            }
          }
          if (m.type === "file_end" && m.id) {
            const item = downloads.current.get(m.id);
            downloads.current.delete(m.id);
            setFsBusy(false);
            if (item) {
              const blob = new Blob(item.chunks as BlobPart[]);
              const url = URL.createObjectURL(blob);
              const a = document.createElement("a");
              a.href = url;
              a.download = item.name;
              a.click();
              URL.revokeObjectURL(url);
              msg.success(`已下载 ${item.name}`);
            }
          }
          if (m.type === "file_error") {
            setError(m.message || "文件传输失败");
          }
        } catch {
          // ignore malformed control message
        }
        return;
      }
      const buf = ev.data as ArrayBuffer;
      if (buf.byteLength <= 24) return;
      const dv = new DataView(buf);
      if (
        dv.getUint8(0) !== 0x49 ||
        dv.getUint8(1) !== 0x44 ||
        dv.getUint8(2) !== 0x56 ||
        dv.getUint8(3) !== 0x46
      ) {
        return;
      }
      const version = dv.getUint16(4, true);
      const flags = dv.getUint16(6, true);
      const seq = dv.getUint32(8, true);
      if (version === 3) {
        const decoder = h264Decoder.current;
        if (!decoder || decoder.state !== "configured") return;
        h264Seq.current = seq;
        try {
          decoder.decode(
            new EncodedVideoChunk({
              type: (flags & 1) !== 0 ? "key" : "delta",
              timestamp: Number(dv.getBigUint64(12, true)) * 1000,
              data: buf.slice(24),
            }),
          );
        } catch {
          h264Decoder.current = null;
          wsRef.current?.send(JSON.stringify({ type: "codec", codec: "jpeg" }));
        }
        return;
      }
      let screenWidth = 0;
      let screenHeight = 0;
      let x = 0;
      let y = 0;
      let width = 0;
      let height = 0;
      let payloadOffset = 20;
      if (version >= 2 && buf.byteLength > 32) {
        screenWidth = dv.getUint16(20, true);
        screenHeight = dv.getUint16(22, true);
        x = dv.getUint16(24, true);
        y = dv.getUint16(26, true);
        width = dv.getUint16(28, true);
        height = dv.getUint16(30, true);
        payloadOffset = 32;
      }
      const bitmap = await createImageBitmap(
        new Blob([buf.slice(payloadOffset)], { type: "image/jpeg" }),
      );
      const canvas = canvasRef.current;
      if (canvas) {
        const nextWidth = screenWidth || bitmap.width;
        const nextHeight = screenHeight || bitmap.height;
        if (canvas.width !== nextWidth || canvas.height !== nextHeight) {
          canvas.width = nextWidth;
          canvas.height = nextHeight;
        }
        const ctx = canvas.getContext("2d");
        if (ctx) {
          if (version < 2 || (flags & 1) !== 0) ctx.clearRect(0, 0, nextWidth, nextHeight);
          if (version >= 2) {
            ctx.drawImage(bitmap, x, y, width || bitmap.width, height || bitmap.height);
          } else {
            ctx.drawImage(bitmap, 0, 0);
          }
        }
      }
      bitmap.close();
      frameCounter.current.frames += 1;
      const now = Date.now();
      if (now - frameCounter.current.at >= 1000) {
        const fps = Math.round(
          (frameCounter.current.frames * 1000) / (now - frameCounter.current.at),
        );
        frameCounter.current = { at: now, frames: 0 };
        setStreamStats((s) => ({ ...s, fps }));
      }
      const ws = wsRef.current;
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "ack", seq, rtt: lastRtt.current }));
      }
    };
    ws.onclose = () => {
      if (wsRef.current === ws) wsRef.current = null;
      h264Decoder.current?.close();
      h264Decoder.current = null;
    };
    ws.onopen = () => {
      if ("VideoDecoder" in window) {
        ws.send(JSON.stringify({ type: "codec", codec: "h264" }));
      }
    };
    const pingTimer = window.setInterval(() => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "ping", t: performance.now() }));
      }
    }, 2000);
    return () => {
      window.clearInterval(pingTimer);
      if (wsRef.current === ws) wsRef.current = null;
      ws.close();
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

  function norm(e: MouseEvent<HTMLCanvasElement>) {
    const rect = e.currentTarget.getBoundingClientRect();
    return {
      x: Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width)),
      y: Math.min(1, Math.max(0, (e.clientY - rect.top) / rect.height)),
    };
  }

  function sendInput(event: unknown) {
    if (!session || !assist) return;
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "input", event }));
      return;
    }
    api.remote.input(session.id, event).catch((e) =>
      setError(String((e as Error).message || e)),
    );
  }

  function onMouseMove(e: MouseEvent<HTMLCanvasElement>) {
    if (!assist) return;
    pendingMove.current = norm(e);
    if (moveTimer.current !== undefined) return;
    moveTimer.current = window.setTimeout(() => {
      moveTimer.current = undefined;
      if (pendingMove.current) sendInput({ type: "mouse_move", ...pendingMove.current });
    }, 16);
  }

  function onMouseButton(e: MouseEvent<HTMLCanvasElement>, down: boolean) {
    if (!assist) return;
    e.preventDefault();
    const p = norm(e);
    pendingMove.current = null;
    const button = e.button === 2 ? "right" : e.button === 1 ? "middle" : "left";
    sendInput({ type: "mouse_move", ...p });
    sendInput({ type: "mouse_click", button, down });
  }

  function onWheel(e: WheelEvent<HTMLCanvasElement>) {
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
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) {
      setFsBusy(true);
      const id = crypto.randomUUID();
      downloads.current.set(id, { name: entry.name, chunks: [] });
      ws.send(JSON.stringify({ type: "file_get", id, path: entry.path }));
      return;
    }
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
    const target = path.endsWith("\\") ? `${path}${file.name}` : `${path}\\${file.name}`;
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) {
      setFsBusy(true);
      const id = crypto.randomUUID();
      const bytes = new Uint8Array(await file.arrayBuffer());
      ws.send(JSON.stringify({ type: "file_put_start", id, path: target, size: bytes.length }));
      const chunk = 128 * 1024;
      for (let off = 0; off < bytes.length; off += chunk) {
        const part = bytes.subarray(off, Math.min(off + chunk, bytes.length));
        const hex = Array.from(part, (b) => b.toString(16).padStart(2, "0")).join("");
        ws.send(JSON.stringify({ type: "file_chunk", id, offset: off, data: hex }));
      }
      ws.send(JSON.stringify({ type: "file_end", id }));
      setFsBusy(false);
      msg.success(`已上传 ${file.name}`);
      return;
    }
    setFsBusy(true);
    try {
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
        <span className="muted">
          {streamStats.codec} · {streamStats.fps} fps · {streamStats.rtt} ms
        </span>
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
          <canvas
            ref={canvasRef}
            className="assist-frame"
            onMouseMove={onMouseMove}
            onMouseDown={(e) => onMouseButton(e, true)}
            onMouseUp={(e) => onMouseButton(e, false)}
            onWheel={onWheel}
            onContextMenu={(e) => e.preventDefault()}
            style={{
              cursor: assist ? "crosshair" : "default",
              width: "100%",
              height: "100%",
            }}
          />
          {!session ? <p className="muted">会话准备中</p> : null}
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
                    onClick={() => {
                      if (!session) return;
                      const ws = wsRef.current;
                      if (ws && ws.readyState === WebSocket.OPEN) {
                        ws.send(JSON.stringify({ type: "clipboard_get" }));
                        msg.success("已请求读取客户端剪贴板");
                      } else {
                        api.remote
                          .clipboardRead(session.id)
                          .then(() => msg.success("已请求读取客户端剪贴板"))
                          .catch((e) => setError(String(e.message || e)));
                      }
                    }}
                  >
                    读取客户端
                  </button>
                  <button
                    className="btn sm"
                    type="button"
                    disabled={!session}
                    onClick={() => {
                      if (!session) return;
                      const ws = wsRef.current;
                      if (ws && ws.readyState === WebSocket.OPEN) {
                        ws.send(JSON.stringify({ type: "clipboard_set", text: clipboard }));
                        msg.success("已发送到客户端");
                      } else {
                        api.remote
                          .clipboardSet(session.id, clipboard)
                          .then(() => msg.success("已发送到客户端"))
                          .catch((e) => setError(String(e.message || e)));
                      }
                    }}
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
