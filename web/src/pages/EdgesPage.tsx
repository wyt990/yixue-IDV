import { FormEvent, useEffect, useState } from "react";
import {
  api,
  type EdgeNode,
  type ImageItem,
  type ImageReplica,
} from "../api";
import { Modal } from "../components/Modal";
import { useMessage } from "../message";

function formatBytes(n: number) {
  if (!Number.isFinite(n) || n <= 0) return "0 B";
  if (n < 1024) return `${Math.round(n)} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatRate(bps: number) {
  if (!Number.isFinite(bps) || bps <= 0) return "0 B/s";
  if (bps < 1024) return `${Math.round(bps)} B/s`;
  if (bps < 1024 * 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${(bps / (1024 * 1024)).toFixed(1)} MB/s`;
}

function formatHostLoad(edge: EdgeNode) {
  if (!edge.online) return "—";
  const cpu = Number(edge.cpu_pct ?? 0);
  const memUsed = Number(edge.mem_used_bytes ?? 0);
  const memTotal = Number(edge.mem_total_bytes ?? 0);
  const rx = Number(edge.net_rx_bps ?? 0);
  const tx = Number(edge.net_tx_bps ?? 0);
  const mem =
    memTotal > 0
      ? `${formatBytes(memUsed)}/${formatBytes(memTotal)}`
      : formatBytes(memUsed);
  return `CPU ${cpu.toFixed(0)}% · 内存 ${mem} · ↓${formatRate(rx)} / ↑${formatRate(tx)}`;
}

export function EdgesPage() {
  const msg = useMessage();
  const [edges, setEdges] = useState<EdgeNode[]>([]);
  const [images, setImages] = useState<ImageItem[]>([]);
  const [replicas, setReplicas] = useState<ImageReplica[]>([]);
  const [selectedEdge, setSelectedEdge] = useState("");
  const [selectedImage, setSelectedImage] = useState("");
  const [name, setName] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [detailEdge, setDetailEdge] = useState<EdgeNode | null>(null);
  const [editName, setEditName] = useState("");
  const [editCaptureTmp, setEditCaptureTmp] = useState("");
  const [formError, setFormError] = useState("");
  const [showToken, setShowToken] = useState(false);

  async function refreshList() {
    const e = await api.edges.list();
    setEdges(e);
    if (!selectedEdge && e[0]) setSelectedEdge(e[0].id);
    setDetailEdge((prev) => {
      if (!prev) return prev;
      return e.find((x) => x.id === prev.id) ?? prev;
    });
  }

  async function refresh() {
    const [e, imgs] = await Promise.all([api.edges.list(), api.images.list()]);
    setEdges(e);
    setImages(imgs);
    if (!selectedEdge && e[0]) setSelectedEdge(e[0].id);
    if (!selectedImage && imgs[0]) setSelectedImage(imgs[0].id);
    const edgeId = selectedEdge || e[0]?.id;
    if (edgeId) setReplicas(await api.edges.replicas(edgeId));
    setDetailEdge((prev) => {
      if (!prev) return prev;
      return e.find((x) => x.id === prev.id) ?? prev;
    });
  }

  useEffect(() => {
    refresh().catch((err) => msg.error(err.message));
    const timer = setInterval(() => {
      refreshList().catch(() => undefined);
    }, 2500);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!selectedEdge) return;
    api.edges
      .replicas(selectedEdge)
      .then(setReplicas)
      .catch((err) => msg.error(err.message));
  }, [selectedEdge]);

  function openCreate() {
    setName("");
    setFormError("");
    setCreateOpen(true);
  }

  async function createEdge(e: FormEvent) {
    e.preventDefault();
    setFormError("");
    try {
      const edge = await api.edges.create(name);
      setCreateOpen(false);
      msg.success("已创建边缘节点，请复制 token 写入 edge.toml");
      await refresh();
      setSelectedEdge(edge.id);
      openDetail(edge);
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "创建失败");
    }
  }

  function openDetail(edge: EdgeNode) {
    setDetailEdge(edge);
    setEditName(edge.name);
    setEditCaptureTmp(edge.capture_tmp_dir ?? "");
    setShowToken(false);
    setFormError("");
  }

  async function copyToken(token: string) {
    try {
      await navigator.clipboard.writeText(token);
      msg.success("已复制完整 token 到剪贴板");
    } catch {
      // fallback for older browsers / insecure context
      const ta = document.createElement("textarea");
      ta.value = token;
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      document.body.removeChild(ta);
      msg.success("已复制完整 token 到剪贴板");
    }
  }

  async function saveEdge(e: FormEvent) {
    e.preventDefault();
    if (!detailEdge) return;
    setFormError("");
    try {
      const updated = await api.edges.update(
        detailEdge.id,
        editName,
        editCaptureTmp.trim(),
      );
      setDetailEdge(updated);
      setEditCaptureTmp(updated.capture_tmp_dir ?? "");
      msg.success(
        "已保存；采集暂存目录将在 Edge 下次心跳（约数秒）后生效",
      );
      await refresh();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "更新失败");
    }
  }

  async function rotateDetailToken() {
    if (!detailEdge) return;
    if (
      !window.confirm(
        `轮换「${detailEdge.name}」的 token？旧 token 将立即失效，需更新 edge.toml。`,
      )
    ) {
      return;
    }
    setFormError("");
    try {
      const edge = await api.edges.rotateToken(detailEdge.id);
      setDetailEdge(edge);
      setShowToken(true);
      msg.success("令牌已轮换，请重新复制到 edge.toml");
      await refresh();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "轮换失败");
    }
  }

  async function syncNow(purgeFirst: boolean) {
    try {
      await api.edges.sync(selectedEdge, selectedImage, purgeFirst);
      msg.success(purgeFirst ? "已下发清空并同步" : "已下发同步任务");
      await refresh();
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "同步失败");
    }
  }

  async function purgeSelected() {
    try {
      await api.edges.purgeReplica(selectedEdge, selectedImage);
      msg.success("已下发清空缓存");
      await refresh();
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "清空失败");
    }
  }

  async function purgeAll() {
    const edge = edges.find((e) => e.id === selectedEdge);
    if (!edge) return;
    const typed = window.prompt(
      `危险操作：清空 Edge「${edge.name}」全部镜像缓存。请输入节点名称确认：`,
    );
    if (typed !== edge.name) {
      msg.warning("未确认节点名，已取消整库清空");
      return;
    }
    try {
      await api.edges.purgeAll(selectedEdge);
      msg.success("已下发整库清空");
      await refresh();
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "整库清空失败");
    }
  }

  async function removeEdge(edge: EdgeNode) {
    if (!window.confirm(`删除边缘节点「${edge.name}」？其副本记录也会删除。`)) {
      return;
    }
    try {
      await api.edges.delete(edge.id);
      if (selectedEdge === edge.id) setSelectedEdge("");
      await refresh();
      msg.success("已删除边缘节点");
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "删除失败");
    }
  }

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div className="panel-head">
        <div>
          <h1>边缘节点</h1>
          <p className="muted">
            注册 Edge、查看在线状态，并将中心镜像同步到边缘缓存。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={openCreate}>
            登记 Edge
          </button>
        </div>
      </div>
      <div className="card">
        <div className="panel-head">
          <h2>节点列表</h2>
          <span className="muted">{edges.length} 个 · 约 2.5 秒自动刷新</span>
        </div>
        <table>
          <thead>
            <tr>
              <th>名称</th>
              <th>状态</th>
              <th>节点负载</th>
              <th>listen</th>
              <th>token</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {edges.length === 0 ? (
              <tr>
                <td colSpan={6} className="muted">
                  暂无边缘节点，点击右上角「登记 Edge」。
                </td>
              </tr>
            ) : (
              edges.map((edge) => (
                <tr key={edge.id}>
                  <td>
                    <button
                      type="button"
                      className="btn ghost"
                      onClick={() => setSelectedEdge(edge.id)}
                    >
                      {edge.name}
                    </button>
                  </td>
                  <td>
                    <span className="tag">
                      {edge.online ? "在线" : "离线"}
                    </span>
                  </td>
                  <td className="mono muted" title={formatHostLoad(edge)}>
                    {formatHostLoad(edge)}
                  </td>
                  <td className="mono muted">{edge.listen_url || "—"}</td>
                  <td>
                    <div className="row-actions">
                      <span className="mono muted" title={edge.node_token}>
                        {edge.node_token.slice(0, 8)}…
                      </span>
                      <button
                        type="button"
                        className="btn ghost"
                        onClick={() => copyToken(edge.node_token)}
                      >
                        复制
                      </button>
                    </div>
                  </td>
                  <td>
                    <div className="row-actions">
                      <button
                        type="button"
                        className="btn ghost"
                        onClick={() => openDetail(edge)}
                      >
                        编辑
                      </button>
                      <button
                        type="button"
                        className="btn danger"
                        onClick={() => removeEdge(edge)}
                      >
                        删除
                      </button>
                    </div>
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      <div className="card">
        <h2>同步 / 缓存</h2>
        <div className="form-row">
          <div className="field">
            <label>Edge</label>
            <select
              value={selectedEdge}
              onChange={(e) => setSelectedEdge(e.target.value)}
            >
              {edges.map((edge) => (
                <option key={edge.id} value={edge.id}>
                  {edge.name}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>镜像</label>
            <select
              value={selectedImage}
              onChange={(e) => setSelectedImage(e.target.value)}
            >
              {images.map((img) => (
                <option key={img.id} value={img.id}>
                  {img.name}
                </option>
              ))}
            </select>
          </div>
        </div>
        <div style={{ display: "flex", gap: "0.75rem", flexWrap: "wrap" }}>
          <button
            className="btn accent"
            type="button"
            disabled={!selectedEdge || !selectedImage}
            onClick={() => syncNow(false)}
          >
            同步到边缘
          </button>
          <button
            className="btn"
            type="button"
            disabled={!selectedEdge || !selectedImage}
            onClick={() => syncNow(true)}
          >
            清空并同步
          </button>
          <button
            className="btn danger"
            type="button"
            disabled={!selectedEdge || !selectedImage}
            onClick={purgeSelected}
          >
            清空该镜像缓存
          </button>
          <button
            className="btn danger"
            type="button"
            disabled={!selectedEdge}
            onClick={purgeAll}
          >
            整库清空
          </button>
          <button
            className="btn ghost"
            type="button"
            disabled={!selectedEdge}
            onClick={() => {
              const edge = edges.find((e) => e.id === selectedEdge);
              if (edge) openDetail(edge);
            }}
          >
            编辑 / 轮换令牌
          </button>
        </div>
      </div>

      <div className="card">
        <div className="panel-head">
          <h2>副本状态</h2>
          <span className="muted">{replicas.length} 条</span>
        </div>
        <table>
          <thead>
            <tr>
              <th>镜像</th>
              <th>状态</th>
              <th>进度</th>
              <th>校验和</th>
            </tr>
          </thead>
          <tbody>
            {replicas.map((r) => (
              <tr key={r.id}>
                <td className="mono">{r.image_id.slice(0, 8)}…</td>
                <td>
                  <span className="tag">{r.state}</span>
                </td>
                <td className="mono">{Math.round(r.progress * 100)}%</td>
                <td className="mono muted">
                  {r.checksum_sha256
                    ? `${r.checksum_sha256.slice(0, 12)}…`
                    : "—"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <Modal
        open={createOpen}
        title="登记 Edge"
        onClose={() => setCreateOpen(false)}
      >
        <form onSubmit={createEdge}>
          {formError ? <p className="error">{formError}</p> : null}
          <div className="field">
            <label>名称</label>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              autoFocus
              required
            />
          </div>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setCreateOpen(false)}
            >
              取消
            </button>
            <button className="btn" type="submit">
              创建并生成 token
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        open={!!detailEdge}
        title={detailEdge ? `编辑边缘节点 · ${detailEdge.name}` : "编辑边缘节点"}
        onClose={() => setDetailEdge(null)}
        wide
      >
        {detailEdge ? (
          <form onSubmit={saveEdge}>
            {formError ? <p className="error">{formError}</p> : null}
            <div className="field">
              <label>名称</label>
              <input
                value={editName}
                onChange={(e) => setEditName(e.target.value)}
                required
              />
            </div>
            <div className="field">
              <label>采集暂存目录（Edge 本机路径）</label>
              <input
                className="mono"
                value={editCaptureTmp}
                onChange={(e) => setEditCaptureTmp(e.target.value)}
                placeholder="例如 H:\AI\idv_rust\data\edge-cache\capture-tmp"
              />
              <p className="muted" style={{ margin: "0.35rem 0 0" }}>
                整盘采集会先写到此目录，请选大容量盘。留空则使用 Edge 本地默认（cache_dir/capture-tmp）。保存后约数秒内经心跳下发。
              </p>
            </div>
            <div className="field">
              <label>node_token（写入 edge.toml）</label>
              <div className="page-actions" style={{ marginBottom: "0.5rem" }}>
                <input
                  className="mono"
                  style={{ flex: 1, minWidth: "12rem" }}
                  value={
                    showToken
                      ? detailEdge.node_token
                      : `${detailEdge.node_token.slice(0, 8)}…（点击显示完整）`
                  }
                  readOnly
                  onFocus={() => setShowToken(true)}
                />
                <button
                  type="button"
                  className="btn ghost"
                  onClick={() => setShowToken((v) => !v)}
                >
                  {showToken ? "隐藏" : "显示"}
                </button>
                <button
                  type="button"
                  className="btn accent"
                  onClick={() => copyToken(detailEdge.node_token)}
                >
                  复制完整 token
                </button>
              </div>
              <p className="muted" style={{ margin: 0 }}>
                状态：{detailEdge.online ? "在线" : "离线"} · listen：
                <span className="mono">
                  {detailEdge.listen_url || "尚未上报"}
                </span>
              </p>
            </div>
            <div className="modal-actions">
              <button
                type="button"
                className="btn ghost"
                onClick={() => setDetailEdge(null)}
              >
                关闭
              </button>
              <button
                type="button"
                className="btn ghost"
                onClick={rotateDetailToken}
              >
                轮换令牌
              </button>
              <button className="btn" type="submit">
                保存
              </button>
            </div>
          </form>
        ) : null}
      </Modal>
    </div>
  );
}
