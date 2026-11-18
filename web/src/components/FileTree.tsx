import { useEffect, useRef, useState, type ReactNode } from "react";
import { api } from "../api";

type Entry = {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: number;
};

type TreeNode = Entry & {
  children?: Entry[];
  expanded?: boolean;
  loaded?: boolean;
  loading?: boolean;
};

type MenuState = { x: number; y: number; entry: Entry } | null;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function joinPath(dir: string, name: string) {
  return dir.endsWith("\\") || dir.endsWith("/") ? `${dir}${name}` : `${dir}\\${name}`;
}

function parentPath(path: string) {
  const clean = path.replace(/[\\/]+$/, "");
  const at = Math.max(clean.lastIndexOf("\\"), clean.lastIndexOf("/"));
  if (at < 0) return "";
  const parent = clean.slice(0, at);
  return parent.length === 2 && parent.endsWith(":") ? `${parent}\\` : parent;
}

function baseName(path: string) {
  const clean = path.replace(/[\\/]+$/, "");
  const at = Math.max(clean.lastIndexOf("\\"), clean.lastIndexOf("/"));
  return clean.slice(at + 1) || clean;
}

function isSameOrChild(path: string, parent: string) {
  const p = path.toLowerCase().replace(/[\\/]+$/, "");
  const q = parent.toLowerCase().replace(/[\\/]+$/, "");
  return p === q || p.startsWith(`${q}\\`);
}

function iconKind(name: string) {
  const ext = name.split(".").pop()?.toLowerCase() || "";
  if (["doc", "docx", "wps", "txt", "md", "rtf"].includes(ext)) return "doc";
  if (["xls", "xlsx", "csv", "et"].includes(ext)) return "sheet";
  if (["ppt", "pptx", "dps"].includes(ext)) return "slide";
  if (["pdf"].includes(ext)) return "pdf";
  if (["zip", "rar", "7z", "tar", "gz", "bz2"].includes(ext)) return "zip";
  if (["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "ico"].includes(ext)) return "image";
  if (["mp4", "avi", "mkv", "mov", "wmv", "flv", "webm"].includes(ext)) return "video";
  if (["mp3", "wav", "flac", "aac", "ogg", "m4a"].includes(ext)) return "audio";
  if (["exe", "msi", "bat", "cmd", "ps1", "sh"].includes(ext)) return "exec";
  if (["rs", "ts", "tsx", "js", "jsx", "py", "java", "c", "cpp", "h", "cs", "go", "json", "xml", "html", "css"].includes(ext)) return "code";
  return "file";
}

const ICON_COLORS: Record<string, string> = {
  doc: "#2b6cb0",
  sheet: "#2f855a",
  slide: "#c05621",
  pdf: "#c53030",
  zip: "#805ad5",
  image: "#3182ce",
  video: "#d53f8c",
  audio: "#6b46c1",
  exec: "#4a5568",
  code: "#2d3748",
  file: "#718096",
};

function FileIcon({ name, isDir }: { name: string; isDir: boolean }) {
  if (isDir) {
    return (
      <svg className="file-icon" viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M3 6.5A1.5 1.5 0 0 1 4.5 5h5l2 2h8A1.5 1.5 0 0 1 21 8.5v9A1.5 1.5 0 0 1 19.5 19h-15A1.5 1.5 0 0 1 3 17.5z"
          fill="#e9b949"
        />
        <path d="M3 9h18v8.5A1.5 1.5 0 0 1 19.5 19h-15A1.5 1.5 0 0 1 3 17.5z" fill="#f6c85f" />
      </svg>
    );
  }
  const kind = iconKind(name);
  const color = ICON_COLORS[kind] || ICON_COLORS.file;
  const label = kind === "file" ? (name.split(".").pop() || "?").slice(0, 3).toUpperCase() : kind.slice(0, 3).toUpperCase();
  return (
    <svg className="file-icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M6 2.5h8l4 4V21a1.5 1.5 0 0 1-1.5 1.5h-10A1.5 1.5 0 0 1 5 21V4A1.5 1.5 0 0 1 6.5 2.5z" fill="#fff" stroke="#cbd5e0" />
      <path d="M14 2.5v4h4" fill="none" stroke="#cbd5e0" />
      <rect x="7" y="10" width="10" height="8" rx="1" fill={color} opacity="0.9" />
      <text x="12" y="15.7" textAnchor="middle" fontSize="4.5" fill="#fff" fontFamily="system-ui, sans-serif">
        {label}
      </text>
    </svg>
  );
}

export function FileTree({
  sessionId,
  busy,
  onBusy,
  onError,
  onUpload,
  onDownload,
}: {
  sessionId: string;
  busy: boolean;
  onBusy: (value: boolean) => void;
  onError: (message: string) => void;
  onUpload: (file: File, targetDir: string) => Promise<void>;
  onDownload: (entry: Entry, name?: string, forceHttp?: boolean) => Promise<void>;
}) {
  const [nodes, setNodes] = useState<TreeNode[]>([]);
  const [selected, setSelected] = useState("C:\\");
  const [selectedIsDir, setSelectedIsDir] = useState(true);
  const [menu, setMenu] = useState<MenuState>(null);
  const [loadingRoot, setLoadingRoot] = useState(true);
  const uploadInput = useRef<HTMLInputElement | null>(null);
  const uploadDir = useRef("C:\\");
  const dragSource = useRef("");

  async function waitFs(requestId: string) {
    const started = Date.now();
    while (Date.now() - started < 60000) {
      const st = await api.remote.fsStatus(requestId);
      if (st.state === "done" || st.state === "failed") return st;
      await sleep(400);
    }
    throw new Error("文件操作超时");
  }

  async function fetchEntries(path: string): Promise<Entry[]> {
    const { request_id } = await api.remote.fsList(sessionId, path);
    const st = await waitFs(request_id);
    if (st.state === "failed") throw new Error(st.error_message || "列目录失败");
    const parsed = st.payload ? JSON.parse(st.payload) : { entries: [] };
    return (parsed.entries || []) as Entry[];
  }

  function updateNode(path: string, updater: (node: TreeNode) => TreeNode, list = nodes, set = setNodes) {
    const walk = (items: TreeNode[]): TreeNode[] =>
      items.map((item) => {
        if (item.path.toLowerCase() === path.toLowerCase()) return updater(item);
        if (item.children) return { ...item, children: walk(item.children) };
        return item;
      });
    set(walk(list));
  }

  async function loadRoots() {
    setLoadingRoot(true);
    try {
      const entries = await fetchEntries("");
      const roots: TreeNode[] = entries.map((e) => ({ ...e, expanded: false, loaded: false }));
      setNodes(roots);
      if (roots.length) {
        setSelected(roots[0].path);
        if (roots[0].is_dir) await loadChildren(roots[0].path, roots);
      }
    } catch (e) {
      onError(String((e as Error).message || e));
    } finally {
      setLoadingRoot(false);
    }
  }

  async function loadChildren(path: string, base?: TreeNode[]) {
    updateNode(path, (n) => ({ ...n, loading: true }), base);
    try {
      const entries = await fetchEntries(path);
      updateNode(
        path,
        (n) => ({ ...n, children: entries, loaded: true, loading: false, expanded: true }),
        base,
      );
    } catch (e) {
      updateNode(path, (n) => ({ ...n, loading: false }), base);
      onError(String((e as Error).message || e));
    }
  }

  async function toggle(node: TreeNode) {
    setSelected(node.path);
    if (!node.is_dir || loadingRoot) return;
    if (node.expanded) {
      updateNode(node.path, (n) => ({ ...n, expanded: false }));
      return;
    }
    if (!node.loaded) {
      await loadChildren(node.path);
    } else {
      updateNode(node.path, (n) => ({ ...n, expanded: true }));
    }
  }

  async function refreshDir(path: string) {
    try {
      const entries = await fetchEntries(path);
      updateNode(path, (n) => ({ ...n, children: entries, loaded: true, expanded: true }));
    } catch {
      // Refresh is best-effort; the error is already visible through the tree state.
    }
  }

  async function moveItem(source: string, targetDir: string) {
    const target = joinPath(targetDir, baseName(source));
    if (isSameOrChild(targetDir, source)) {
      onError("不能把目录移动到自身或子目录");
      return;
    }
    if (source.toLowerCase() === target.toLowerCase()) return;
    onBusy(true);
    try {
      const { request_id } = await api.remote.fsMove(sessionId, source, target);
      const st = await waitFs(request_id);
      if (st.state === "failed") throw new Error(st.error_message || "移动失败");
      await refreshDir(parentPath(source));
      await refreshDir(targetDir);
      setSelected(targetDir);
    } catch (e) {
      onError(String((e as Error).message || e));
    } finally {
      onBusy(false);
    }
  }

  async function dropFiles(files: FileList, targetDir: string) {
    if (!files.length) return;
    for (const file of Array.from(files)) {
      await onUpload(file, targetDir);
    }
    await refreshDir(targetDir);
  }

  function askUpload(dir: string) {
    uploadDir.current = dir;
    uploadInput.current?.click();
  }

  useEffect(() => {
    loadRoots();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const scroll = () => setMenu(null);
    window.addEventListener("click", close);
    window.addEventListener("scroll", scroll, true);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("scroll", scroll, true);
    };
  }, [menu]);

  function renderRows(items: TreeNode[], depth = 0): ReactNode[] {
    const rows: ReactNode[] = [];
    for (const item of items) {
      rows.push(
        <div
          key={item.path}
          className={`tree-row${selected === item.path ? " selected" : ""}`}
          style={{ paddingLeft: `${0.4 + depth * 1.05}rem` }}
          draggable
          onDragStart={(e) => {
            dragSource.current = item.path;
            e.dataTransfer.effectAllowed = "move";
            e.dataTransfer.setData("application/x-idv-path", item.path);
            e.dataTransfer.setData("text/plain", item.path);
          }}
          onClick={() => {
            setSelected(item.path);
            setSelectedIsDir(item.is_dir);
          }}
          onDoubleClick={() => {
            if (item.is_dir) toggle(item);
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setSelected(item.path);
            setSelectedIsDir(item.is_dir);
            setMenu({
              x: Math.min(e.clientX, window.innerWidth - 190),
              y: Math.min(e.clientY, window.innerHeight - 180),
              entry: item,
            });
          }}
          onDragOver={(e) => {
            if (!item.is_dir) return;
            e.preventDefault();
            e.dataTransfer.dropEffect = dragSource.current ? "move" : "copy";
          }}
          onDrop={(e) => {
            if (!item.is_dir) return;
            e.preventDefault();
            e.stopPropagation();
            if (e.dataTransfer.files.length) {
              dropFiles(e.dataTransfer.files, item.path);
              return;
            }
            const source = e.dataTransfer.getData("application/x-idv-path") || dragSource.current;
            if (source) moveItem(source, item.path);
          }}
        >
          <button
            type="button"
            className={`tree-twisty${item.is_dir ? "" : " empty"}`}
            onClick={(e) => {
              e.stopPropagation();
              if (item.is_dir) toggle(item);
            }}
            aria-label={item.expanded ? "折叠" : "展开"}
          >
            {item.is_dir ? (item.loading ? "…" : item.expanded ? "▾" : "▸") : ""}
          </button>
          <FileIcon name={item.name} isDir={item.is_dir} />
          <span className="tree-name" title={item.path}>
            {item.name}
          </span>
          <span className="tree-meta">
            {item.is_dir ? "" : `${Math.max(1, Math.round(item.size / 1024))} KB`}
          </span>
        </div>,
      );
      if (item.is_dir && item.expanded && item.children?.length) {
        rows.push(...renderRows(item.children.map((child) => ({ ...child })), depth + 1));
      }
    }
    return rows;
  }

  return (
    <div className="assist-tree-panel">
      <div className="assist-tree-toolbar">
        <button className="btn ghost sm" type="button" disabled={busy} onClick={loadRoots}>
          刷新盘符
        </button>
        <label className="btn ghost sm">
          上传到当前目录
          <input
            type="file"
            multiple
            hidden
            onChange={async (e) => {
              const files = e.target.files;
              if (files?.length) await dropFiles(files, selectedIsDir ? selected : parentPath(selected));
              e.currentTarget.value = "";
            }}
          />
        </label>
        <span className="muted tree-selected" title={selected}>
          {selected}
        </span>
      </div>
      <div
        className="assist-file-tree"
        onContextMenu={(e) => e.preventDefault()}
        onDragOver={(e) => {
          if (e.dataTransfer.files.length) e.preventDefault();
        }}
        onDrop={(e) => {
          if (e.dataTransfer.files.length) {
            e.preventDefault();
            dropFiles(e.dataTransfer.files, selectedIsDir ? selected : parentPath(selected));
          }
        }}
      >
        {loadingRoot ? (
          <p className="muted">正在读取盘符…</p>
        ) : nodes.length === 0 ? (
          <p className="muted">没有可访问的盘符</p>
        ) : (
          renderRows(nodes)
        )}
      </div>
      <input
        ref={uploadInput}
        type="file"
        multiple
        hidden
        onChange={async (e) => {
          const files = e.target.files;
          if (files?.length) await dropFiles(files, uploadDir.current);
          e.currentTarget.value = "";
        }}
      />
      {menu ? (
        <div
          className="assist-context-menu"
          style={{ left: menu.x, top: menu.y }}
          onClick={(e) => e.stopPropagation()}
        >
          {menu.entry.is_dir ? (
            <>
              <button
                type="button"
                onClick={() => {
                  setMenu(null);
                  onDownload(menu.entry, `${menu.entry.name}.zip`, true);
                }}
              >
                压缩后下载
              </button>
              <button
                type="button"
                onClick={() => {
                  setMenu(null);
                  askUpload(menu.entry.path);
                }}
              >
                上传到此目录
              </button>
            </>
          ) : (
            <button
              type="button"
              onClick={() => {
                setMenu(null);
                onDownload(menu.entry);
              }}
            >
              下载
            </button>
          )}
          <button
            type="button"
            onClick={() => {
              navigator.clipboard?.writeText(menu.entry.path).then(
                () => onError(""),
                () => onError("复制路径失败"),
              );
              setMenu(null);
            }}
          >
            复制路径
          </button>
          <button
            type="button"
            onClick={() => {
              setMenu(null);
              refreshDir(menu.entry.is_dir ? menu.entry.path : parentPath(menu.entry.path));
            }}
          >
            刷新
          </button>
        </div>
      ) : null}
    </div>
  );
}
