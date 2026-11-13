import { FormEvent, useEffect, useMemo, useState } from "react";
import {
  api,
  type Campus,
  type Client,
  type EdgeNode,
  type ImageItem,
  type Lab,
  type TaskLifecycleSettings,
  type TaskView,
} from "../api";
import { Modal } from "../components/Modal";
import { useMessage } from "../message";
import {
  diskPlanLabel,
  taskKindLabel,
  taskStateLabel,
} from "../taskLabels";

const DISK_PLANS = [
  {
    id: "wipe_whole_disk",
    label: "整盘覆盖",
    hint: "重建整盘并写入镜像全部内容",
  },
  {
    id: "wipe_target_partitions",
    label: "仅系统分区",
    hint: "覆盖 role=system 分区，尽量保留数据区",
  },
  {
    id: "write_system_only",
    label: "仅写系统区",
    hint: "与现网分区布局兼容时仅写 system",
  },
] as const;

type ModalKind = "capture" | "deploy" | "lifecycle" | null;

export function TasksPage() {
  const msg = useMessage();
  const [tasks, setTasks] = useState<TaskView[]>([]);
  const [images, setImages] = useState<ImageItem[]>([]);
  const [edges, setEdges] = useState<EdgeNode[]>([]);
  const [clients, setClients] = useState<Client[]>([]);
  const [campuses, setCampuses] = useState<Campus[]>([]);
  const [labs, setLabs] = useState<Lab[]>([]);
  const [modal, setModal] = useState<ModalKind>(null);
  const [formError, setFormError] = useState("");

  const [captureMac, setCaptureMac] = useState("");
  const [captureName, setCaptureName] = useState("");
  const [excludeTemp, setExcludeTemp] = useState(true);
  const [deployImage, setDeployImage] = useState("");
  const [selectedMacs, setSelectedMacs] = useState<Set<string>>(new Set());
  const [filterCampusId, setFilterCampusId] = useState("");
  const [filterLabId, setFilterLabId] = useState("");
  const [diskPlan, setDiskPlan] = useState("wipe_whole_disk");
  const [edgeId, setEdgeId] = useState("");
  const [lifecycle, setLifecycle] = useState<TaskLifecycleSettings>({
    stale_after_secs: 12 * 3600,
  });
  const [staleHours, setStaleHours] = useState("12");

  const labMap = useMemo(
    () => new Map(labs.map((l) => [l.id, l])),
    [labs],
  );
  const campusMap = useMemo(
    () => new Map(campuses.map((c) => [c.id, c])),
    [campuses],
  );
  const labsForFilter = useMemo(() => {
    if (!filterCampusId) return labs;
    return labs.filter((l) => l.campus_id === filterCampusId);
  }, [labs, filterCampusId]);
  const filteredClients = useMemo(() => {
    return clients.filter((c) => {
      const lab = labMap.get(c.lab_id);
      if (filterLabId && c.lab_id !== filterLabId) return false;
      if (filterCampusId && lab?.campus_id !== filterCampusId) return false;
      return true;
    });
  }, [clients, labMap, filterCampusId, filterLabId]);
  const filteredAllSelected =
    filteredClients.length > 0 &&
    filteredClients.every((c) => selectedMacs.has(c.mac));
  const selectedMacList = useMemo(
    () => Array.from(selectedMacs),
    [selectedMacs],
  );

  async function refreshTasks() {
    setTasks(await api.tasks.list());
  }

  async function refresh() {
    const [t, imgs, e, c, camps, ls] = await Promise.all([
      api.tasks.list(),
      api.images.list(),
      api.edges.list(),
      api.clients.list(),
      api.campuses.list(),
      api.labs.list(),
    ]);
    setTasks(t);
    setImages(imgs);
    setEdges(e);
    setClients(c);
    setCampuses(camps);
    setLabs(ls);
    if (!deployImage && imgs[0]) setDeployImage(imgs[0].id);
    if (!captureMac && c[0]) setCaptureMac(c[0].mac);
  }

  function toggleMac(mac: string, on: boolean) {
    setSelectedMacs((prev) => {
      const next = new Set(prev);
      if (on) next.add(mac);
      else next.delete(mac);
      return next;
    });
  }

  function toggleFiltered(on: boolean) {
    setSelectedMacs((prev) => {
      const next = new Set(prev);
      for (const c of filteredClients) {
        if (on) next.add(c.mac);
        else next.delete(c.mac);
      }
      return next;
    });
  }

  function selectCurrentLab() {
    if (!filterLabId) return;
    setSelectedMacs((prev) => {
      const next = new Set(prev);
      for (const c of clients) {
        if (c.lab_id === filterLabId) next.add(c.mac);
      }
      return next;
    });
  }

  function openModal(kind: ModalKind) {
    setFormError("");
    if (kind === "deploy" && selectedMacList.length > 0) {
      const resolved = selectedMacList.map((mac) => {
        const client = clients.find((c) => c.mac === mac);
        if (!client) return null;
        const lab = labs.find((l) => l.id === client.lab_id);
        if (!lab) return null;
        if (lab.default_image_id) return lab.default_image_id;
        const campus = campuses.find((c) => c.id === lab.campus_id);
        return campus?.default_image_id || null;
      });
      const unique = [...new Set(resolved.filter(Boolean))] as string[];
      if (unique.length === 1) setDeployImage(unique[0]);
    }
    setModal(kind);
  }

  useEffect(() => {
    refresh().catch((err) => msg.error(err.message));
    api.taskLifecycle
      .get()
      .then((life) => {
        setLifecycle(life);
        setStaleHours(
          life.stale_after_secs <= 0
            ? "0"
            : String(Math.round(life.stale_after_secs / 3600)),
        );
      })
      .catch((err) => msg.error(err.message));
    // Only poll task list (progress); avoid full refresh failing the poll.
    const timer = setInterval(() => {
      refreshTasks().catch(() => undefined);
    }, 1500);
    return () => clearInterval(timer);
  }, []);

  async function onCapture(e: FormEvent) {
    e.preventDefault();
    setFormError("");
    try {
      if (!captureMac) {
        setFormError("请选择金样机");
        return;
      }
      await api.tasks.capture({
        mac: captureMac,
        name: captureName || undefined,
        edge_id: edgeId || undefined,
        exclude_temp: excludeTemp,
      });
      setCaptureName("");
      setModal(null);
      msg.success(
        excludeTemp
          ? "已创建采集任务（将清除 pagefile/Temp 等后再打包）"
          : "已创建采集任务（未启用排除）",
      );
      await refresh();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "创建失败");
    }
  }

  async function onCancelTask(id: string) {
    try {
      await api.tasks.cancel(id);
      msg.success("已结束任务");
      await refresh();
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "结束失败");
    }
  }

  async function onSaveLifecycle(e: FormEvent) {
    e.preventDefault();
    setFormError("");
    try {
      const hours = Number(staleHours);
      if (!Number.isFinite(hours) || hours < 0) {
        setFormError("超时小时数须为 ≥ 0 的数字（0=关闭自动收尸）");
        return;
      }
      const next = await api.taskLifecycle.set({
        stale_after_secs: Math.round(hours * 3600),
      });
      setLifecycle(next);
      setModal(null);
      msg.success(
        next.stale_after_secs <= 0
          ? "已关闭任务自动收尸"
          : `已保存：${Math.round(next.stale_after_secs / 3600)} 小时无进度则自动结束`,
      );
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "保存失败");
    }
  }

  async function onDeploy(e: FormEvent) {
    e.preventDefault();
    setFormError("");
    if (selectedMacList.length === 0) {
      setFormError("请先勾选至少一台目标客户端");
      return;
    }
    if (!deployImage) {
      setFormError("请选择镜像");
      return;
    }
    try {
      await api.tasks.deploy({
        image_id: deployImage,
        macs: selectedMacList,
        disk_plan: diskPlan,
        edge_id: edgeId || undefined,
        confirm_destructive: true,
      });
      setSelectedMacs(new Set());
      setModal(null);
      msg.success(`已创建部署任务（${selectedMacList.length} 台）`);
      await refresh();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "创建失败");
    }
  }

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div className="panel-head">
        <div>
          <h1>维护任务</h1>
          <p className="muted">
            采集入库与批量部署；区分「同步到边缘」与「部署到终端」。
          </p>
        </div>
        <div className="page-actions">
          <button
            type="button"
            className="btn"
            onClick={() => openModal("capture")}
          >
            采集任务
          </button>
          <button
            type="button"
            className="btn accent"
            onClick={() => openModal("deploy")}
          >
            部署任务
          </button>
          <button
            type="button"
            className="btn ghost"
            onClick={() => openModal("lifecycle")}
          >
            超时设置
          </button>
        </div>
      </div>

      <div className="card">
        <div className="panel-head">
          <div>
            <h2>任务列表</h2>
            <p className="muted" style={{ margin: 0 }}>
              进度约 1.5 秒自动刷新，无需手动刷新页面
            </p>
          </div>
          <button
            className="btn ghost"
            type="button"
            onClick={() =>
              api.tasks
                .exportCsv()
                .then(() => msg.success("已开始导出 CSV"))
                .catch((err) => msg.error(err.message))
            }
          >
            导出 CSV
          </button>
        </div>
        <table>
          <thead>
            <tr>
              <th>名称</th>
              <th>类型</th>
              <th>状态</th>
              <th>进度</th>
              <th>策略</th>
              <th>目标</th>
              <th>更新</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {tasks.map((t) => {
              const progressTarget =
                t.targets.find(
                  (tg) =>
                    (tg.state === "running" || tg.state === "pending") &&
                    tg.detail,
                ) ||
                t.targets.find((tg) => tg.detail) ||
                t.targets[0];
              const progressText =
                progressTarget?.detail &&
                /\d+(\.\d+)?\s*\/\s*(\d+(\.\d+)?|\?)\s*GB/i.test(
                  progressTarget.detail,
                )
                  ? progressTarget.detail.split("·")[0].trim()
                  : progressTarget && progressTarget.progress > 0
                    ? `${Math.round(progressTarget.progress * 100)}%`
                    : "—";
              const macSummary = t.targets.length
                ? `${t.targets.map((tg) => tg.mac).join(", ")} · ${
                    t.targets.length === 1
                      ? taskStateLabel(t.targets[0].state)
                      : `${t.targets.length} 台`
                  }`
                : "—";
              const err = t.targets
                .map((tg) => tg.error_message)
                .filter(Boolean)
                .join("; ");
              return (
                <tr key={t.id}>
                  <td>{t.name || t.id.slice(0, 8)}</td>
                  <td>{taskKindLabel(t.kind)}</td>
                  <td>{taskStateLabel(t.state)}</td>
                  <td className="mono">{progressText}</td>
                  <td className="muted">{diskPlanLabel(t.disk_plan)}</td>
                  <td className="mono" title={err || undefined}>
                    {macSummary}
                    {err ? ` · ${err}` : ""}
                  </td>
                  <td className="muted">
                    {new Date(t.updated_at).toLocaleString()}
                  </td>
                  <td>
                    {t.state === "pending" || t.state === "running" ? (
                      <button
                        className="btn ghost"
                        type="button"
                        onClick={() => onCancelTask(t.id)}
                      >
                        结束
                      </button>
                    ) : null}
                  </td>
                </tr>
              );
            })}
            {tasks.length === 0 ? (
              <tr>
                <td colSpan={8} className="muted">
                  暂无任务
                </td>
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>

      <Modal
        open={modal === "capture"}
        title="采集任务"
        onClose={() => setModal(null)}
      >
        <form onSubmit={onCapture}>
          {formError ? <p className="error">{formError}</p> : null}
          <div className="field">
            <label>金样机</label>
            <select
              value={captureMac}
              onChange={(e) => {
                const mac = e.target.value;
                setCaptureMac(mac);
                const client = clients.find((c) => c.mac === mac);
                if (client && !captureName.trim()) {
                  setCaptureName(`${client.name}-金样`);
                }
              }}
              required
            >
              <option value="" disabled>
                {clients.length === 0
                  ? "暂无终端，请先 PXE 报到登记"
                  : "请选择终端"}
              </option>
              {clients.map((c) => (
                <option key={c.id} value={c.mac}>
                  {c.name}（{c.mac}）
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>任务/镜像名</label>
            <input
              value={captureName}
              onChange={(e) => setCaptureName(e.target.value)}
              placeholder="机房A-金样"
            />
          </div>
          <div className="field">
            <label>指定 Edge（可选）</label>
            <select value={edgeId} onChange={(e) => setEdgeId(e.target.value)}>
              <option value="">任意</option>
              {edges.map((ed) => (
                <option key={ed.id} value={ed.id}>
                  {ed.name}
                </option>
              ))}
            </select>
          </div>
          <label
            className="muted"
            style={{ display: "flex", gap: "0.5rem", alignItems: "flex-start" }}
          >
            <input
              type="checkbox"
              checked={excludeTemp}
              onChange={(e) => setExcludeTemp(e.target.checked)}
            />
            <span>
              采集前清除临时/大文件（pagefile、hiberfil、Temp、回收站等）。PE
              日志带 [exclude] 前缀，任务进度可见摘要。
            </span>
          </label>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setModal(null)}
            >
              取消
            </button>
            <button className="btn" type="submit">
              下发采集
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        open={modal === "deploy"}
        title="部署任务"
        onClose={() => setModal(null)}
        wide
      >
        <form onSubmit={onDeploy}>
          {formError ? <p className="error">{formError}</p> : null}
          <div className="field">
            <label>镜像</label>
            <select
              value={deployImage}
              onChange={(e) => setDeployImage(e.target.value)}
            >
              {images.length === 0 ? (
                <option value="">暂无镜像</option>
              ) : null}
              {images.map((img) => (
                <option key={img.id} value={img.id}>
                  {img.name}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>目标客户端</label>
            <div className="page-actions" style={{ marginBottom: "0.5rem" }}>
              <select
                value={filterCampusId}
                onChange={(e) => {
                  setFilterCampusId(e.target.value);
                  setFilterLabId("");
                }}
                aria-label="校区筛选"
              >
                <option value="">全部校区</option>
                {campuses.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
              </select>
              <select
                value={filterLabId}
                onChange={(e) => setFilterLabId(e.target.value)}
                aria-label="机房筛选"
              >
                <option value="">全部机房</option>
                {labsForFilter.map((lab) => (
                  <option key={lab.id} value={lab.id}>
                    {lab.name}
                  </option>
                ))}
              </select>
              <span className="muted">
                可见 {filteredClients.length} / 已选 {selectedMacs.size}
              </span>
            </div>
            <div className="page-actions" style={{ marginBottom: "0.5rem" }}>
              <button
                type="button"
                className="btn ghost"
                onClick={() => toggleFiltered(true)}
                disabled={filteredClients.length === 0}
              >
                全选当前筛选
              </button>
              <button
                type="button"
                className="btn ghost"
                onClick={selectCurrentLab}
                disabled={!filterLabId}
              >
                选中本机房全部
              </button>
              <button
                type="button"
                className="btn ghost"
                onClick={() => setSelectedMacs(new Set())}
                disabled={selectedMacs.size === 0}
              >
                清除已选
              </button>
            </div>
            <div style={{ maxHeight: "16rem", overflow: "auto" }}>
              <table>
                <thead>
                  <tr>
                    <th>
                      <input
                        type="checkbox"
                        checked={filteredAllSelected}
                        onChange={(e) => toggleFiltered(e.target.checked)}
                        aria-label="全选当前筛选"
                      />
                    </th>
                    <th>名称</th>
                    <th>MAC</th>
                    <th>机房</th>
                  </tr>
                </thead>
                <tbody>
                  {filteredClients.length === 0 ? (
                    <tr>
                      <td colSpan={4} className="muted">
                        {clients.length === 0
                          ? "暂无终端，请先在「客户端」登记"
                          : "当前筛选无终端"}
                      </td>
                    </tr>
                  ) : (
                    filteredClients.map((c) => {
                      const lab = labMap.get(c.lab_id);
                      const campus = lab
                        ? campusMap.get(lab.campus_id)
                        : undefined;
                      return (
                        <tr key={c.id}>
                          <td>
                            <input
                              type="checkbox"
                              checked={selectedMacs.has(c.mac)}
                              onChange={(e) =>
                                toggleMac(c.mac, e.target.checked)
                              }
                              aria-label={`选择 ${c.name}`}
                            />
                          </td>
                          <td>{c.name}</td>
                          <td className="mono">{c.mac}</td>
                          <td className="muted">
                            {campus ? `${campus.name} / ` : ""}
                            {lab?.name || c.lab_id}
                          </td>
                        </tr>
                      );
                    })
                  )}
                </tbody>
              </table>
            </div>
          </div>
          <div className="field">
            <label>磁盘策略</label>
            <select
              value={diskPlan}
              onChange={(e) => setDiskPlan(e.target.value)}
            >
              {DISK_PLANS.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.label}
                </option>
              ))}
            </select>
          </div>
          <p className="muted">
            {DISK_PLANS.find((p) => p.id === diskPlan)?.hint}
          </p>
          <div className="field">
            <label>指定 Edge（可选）</label>
            <select value={edgeId} onChange={(e) => setEdgeId(e.target.value)}>
              <option value="">任意</option>
              {edges.map((ed) => (
                <option key={ed.id} value={ed.id}>
                  {ed.name}
                </option>
              ))}
            </select>
          </div>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setModal(null)}
            >
              取消
            </button>
            <button
              className="btn"
              type="submit"
              disabled={selectedMacs.size === 0 || !deployImage}
            >
              下发部署（{selectedMacs.size} 台）
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        open={modal === "lifecycle"}
        title="超时设置"
        onClose={() => setModal(null)}
      >
        <form onSubmit={onSaveLifecycle}>
          {formError ? <p className="error">{formError}</p> : null}
          <p className="muted">
            等待中/进行中任务超过设定时长无进度更新则自动标失败。当前：
            {lifecycle.stale_after_secs <= 0
              ? "已关闭"
              : `${Math.round(lifecycle.stale_after_secs / 3600)} 小时`}
            。设为 0 关闭自动。
          </p>
          <div className="field">
            <label>无进度超时（小时）</label>
            <input
              type="number"
              min={0}
              step={1}
              value={staleHours}
              onChange={(e) => setStaleHours(e.target.value)}
            />
          </div>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setModal(null)}
            >
              取消
            </button>
            <button className="btn" type="submit">
              保存
            </button>
          </div>
        </form>
      </Modal>
    </div>
  );
}
