import { FormEvent, useEffect, useRef, useState } from "react";
import { api, type ImageFileEntry, type ImageItem } from "../api";
import { Modal } from "../components/Modal";
import { useMessage } from "../message";

function formatBytes(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function ImagesPage() {
  const msg = useMessage();
  const [images, setImages] = useState<ImageItem[]>([]);
  const [open, setOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<ImageItem | null>(null);
  const [deleteFile, setDeleteFile] = useState(false);
  const [formError, setFormError] = useState("");
  const [busy, setBusy] = useState("");

  const [name, setName] = useState("");
  const [osFamily, setOsFamily] = useState("windows");
  const [version, setVersion] = useState("");
  const [files, setFiles] = useState<ImageFileEntry[]>([]);
  const [selectedFile, setSelectedFile] = useState("");
  const [fileMeta, setFileMeta] = useState<ImageFileEntry | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  async function refresh() {
    setImages(await api.images.list());
  }

  async function refreshFiles() {
    setFiles(await api.images.listFiles());
  }

  useEffect(() => {
    refresh().catch((err) => msg.error(err.message));
  }, []);

  async function openModal() {
    setName("");
    setOsFamily("windows");
    setVersion("");
    setSelectedFile("");
    setFileMeta(null);
    setFormError("");
    setBusy("");
    setOpen(true);
    try {
      await refreshFiles();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "无法列出镜像目录");
    }
  }

  async function onSelectFile(filename: string) {
    setSelectedFile(filename);
    setFileMeta(null);
    setFormError("");
    if (!filename) return;
    setBusy("正在计算 SHA-256…");
    try {
      const meta = await api.images.fileMeta(filename);
      setFileMeta(meta);
      if (!name.trim()) {
        const base = filename.replace(/\.[^.]+$/, "");
        setName(base);
      }
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "读取文件失败");
    } finally {
      setBusy("");
    }
  }

  async function onUpload(file: File | null) {
    if (!file) return;
    setFormError("");
    setBusy(`正在上传 ${file.name}…`);
    try {
      const entry = await api.images.uploadFile(file);
      await refreshFiles();
      setSelectedFile(entry.name);
      setFileMeta(entry);
      if (!name.trim()) {
        setName(file.name.replace(/\.[^.]+$/, ""));
      }
      msg.success(`已上传 ${entry.name}`);
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "上传失败");
    } finally {
      setBusy("");
      if (fileInputRef.current) fileInputRef.current.value = "";
    }
  }

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setFormError("");
    if (!fileMeta?.checksum_sha256) {
      setFormError("请先选择或上传镜像文件");
      return;
    }
    try {
      await api.images.create({
        name,
        os_family: osFamily,
        version_label: version,
        checksum_sha256: fileMeta.checksum_sha256,
        size_bytes: fileMeta.size_bytes,
        storage_uri: fileMeta.storage_uri,
      });
      setOpen(false);
      msg.success("已登记镜像");
      await refresh();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "登记失败");
    }
  }

  function askDelete(img: ImageItem) {
    setPendingDelete(img);
    setDeleteFile(false);
    setFormError("");
    setDeleteOpen(true);
  }

  async function confirmDelete(e: FormEvent) {
    e.preventDefault();
    if (!pendingDelete) return;
    setFormError("");
    try {
      await api.images.delete(pendingDelete.id, deleteFile);
      setDeleteOpen(false);
      setPendingDelete(null);
      msg.success(
        deleteFile
          ? `已删除镜像「${pendingDelete.name}」及文件`
          : `已删除镜像登记「${pendingDelete.name}」`,
      );
      await refresh();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "删除失败");
    }
  }

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div className="panel-head">
        <div>
          <h1>镜像库</h1>
          <p className="muted">
            从中心镜像目录选择或上传文件后登记；校验和由服务端自动计算。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={openModal}>
            登记镜像
          </button>
        </div>
      </div>

      <div className="card">
        <div className="panel-head">
          <h2>已登记镜像</h2>
          <span className="muted">{images.length} 个</span>
        </div>
        <table>
          <thead>
            <tr>
              <th>名称</th>
              <th>系统</th>
              <th>校验和</th>
              <th>路径</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {images.length === 0 ? (
              <tr>
                <td colSpan={5} className="muted">
                  暂无镜像，点击右上角「登记镜像」。
                </td>
              </tr>
            ) : (
              images.map((img) => (
                <tr key={img.id}>
                  <td>
                    {img.name}
                    {img.version_label ? (
                      <div className="muted">{img.version_label}</div>
                    ) : null}
                  </td>
                  <td>
                    <span className={`tag ${img.os_family}`}>
                      {img.os_family}
                    </span>
                  </td>
                  <td className="mono muted">
                    {img.checksum_sha256.slice(0, 16)}…
                  </td>
                  <td className="mono muted">{img.storage_uri || "—"}</td>
                  <td>
                    <button
                      type="button"
                      className="btn danger"
                      onClick={() => askDelete(img)}
                    >
                      删除
                    </button>
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      <Modal
        open={open}
        title="登记镜像"
        onClose={() => setOpen(false)}
        wide
      >
        <form onSubmit={onSubmit}>
          {formError ? <p className="error">{formError}</p> : null}
          {busy ? <p className="muted">{busy}</p> : null}

          <div className="field">
            <label>镜像显示名</label>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="例如：Win11 教学机房基线"
              required
            />
          </div>

          <div className="form-row">
            <div className="field">
              <label>系统族</label>
              <select
                value={osFamily}
                onChange={(e) => setOsFamily(e.target.value)}
              >
                <option value="windows">windows</option>
                <option value="linux">linux</option>
                <option value="other">other</option>
              </select>
            </div>
            <div className="field">
              <label>版本标签</label>
              <input
                value={version}
                onChange={(e) => setVersion(e.target.value)}
                placeholder="可选"
              />
            </div>
          </div>

          <div className="field">
            <label>镜像文件（服务端目录）</label>
            <div className="page-actions" style={{ marginBottom: "0.5rem" }}>
              <select
                value={selectedFile}
                onChange={(e) => onSelectFile(e.target.value)}
                disabled={!!busy}
                style={{ flex: 1, minWidth: "12rem" }}
              >
                <option value="">
                  {files.length === 0
                    ? "目录为空，请先上传或拷贝文件"
                    : "请选择文件…"}
                </option>
                {files.map((f) => (
                  <option key={f.name} value={f.name}>
                    {f.name}（{formatBytes(f.size_bytes)}）
                  </option>
                ))}
              </select>
              <button
                type="button"
                className="btn accent"
                disabled={!!busy}
                onClick={() => fileInputRef.current?.click()}
              >
                上传文件
              </button>
              <button
                type="button"
                className="btn ghost"
                disabled={!!busy}
                onClick={() =>
                  refreshFiles().catch((err) =>
                    setFormError(err.message || "刷新失败"),
                  )
                }
              >
                刷新
              </button>
            </div>
            <input
              ref={fileInputRef}
              type="file"
              accept=".img,.bin,.wim,.iso,.vhdx,.vhd,.raw,.qcow2,.delta"
              hidden
              onChange={(e) => onUpload(e.target.files?.[0] ?? null)}
            />
            <p className="muted" style={{ margin: 0 }}>
              也可将文件直接放到中心 <span className="mono">images_dir</span>{" "}
              后点「刷新」。
            </p>
          </div>

          <div className="field">
            <label>SHA-256（自动）</label>
            <input
              className="mono"
              value={fileMeta?.checksum_sha256 || ""}
              readOnly
              placeholder="选择或上传文件后自动填入"
            />
          </div>

          <div className="form-row">
            <div className="field">
              <label>大小</label>
              <input
                className="mono"
                value={
                  fileMeta ? formatBytes(fileMeta.size_bytes) : ""
                }
                readOnly
                placeholder="—"
              />
            </div>
            <div className="field">
              <label>存储路径</label>
              <input
                className="mono"
                value={fileMeta?.storage_uri || ""}
                readOnly
                placeholder="—"
              />
            </div>
          </div>

          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setOpen(false)}
              disabled={!!busy}
            >
              取消
            </button>
            <button
              className="btn"
              type="submit"
              disabled={!!busy || !fileMeta?.checksum_sha256}
            >
              保存登记
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        open={deleteOpen}
        title="删除镜像"
        onClose={() => {
          setDeleteOpen(false);
          setPendingDelete(null);
        }}
      >
        <form onSubmit={confirmDelete}>
          {formError ? <p className="error">{formError}</p> : null}
          <p>
            确定删除镜像登记「{pendingDelete?.name}」？
          </p>
          <p className="muted mono" style={{ wordBreak: "break-all" }}>
            {pendingDelete?.storage_uri || "—"}
          </p>
          <label
            className="muted"
            style={{ display: "flex", gap: "0.5rem", alignItems: "flex-start" }}
          >
            <input
              type="checkbox"
              checked={deleteFile}
              onChange={(e) => setDeleteFile(e.target.checked)}
            />
            <span>
              同时删除镜像文件（默认仅删除登记记录；勾选后将删除 images_dir
              下对应文件）
            </span>
          </label>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => {
                setDeleteOpen(false);
                setPendingDelete(null);
              }}
            >
              取消
            </button>
            <button className="btn danger" type="submit">
              确认删除
            </button>
          </div>
        </form>
      </Modal>
    </div>
  );
}
