import { FormEvent, useEffect, useMemo, useState } from "react";
import {
  api,
  type Campus,
  type Client,
  type EnrollmentSettings,
  type ImageItem,
  type Lab,
} from "../api";
import { Modal } from "../components/Modal";
import { useMessage } from "../message";

function resolveDefaultImageId(
  labId: string,
  labs: Lab[],
  campuses: Campus[],
): string | null {
  const lab = labs.find((l) => l.id === labId);
  if (!lab) return null;
  if (lab.default_image_id) return lab.default_image_id;
  const campus = campuses.find((c) => c.id === lab.campus_id);
  return campus?.default_image_id || null;
}

const DISK_PLANS = [
  { id: "wipe_whole_disk", label: "整盘覆盖" },
  { id: "wipe_target_partitions", label: "仅系统分区" },
  { id: "write_system_only", label: "仅写系统区" },
] as const;

export function ClientsPage() {
  const msg = useMessage();
  const [labs, setLabs] = useState<Lab[]>([]);
  const [campuses, setCampuses] = useState<Campus[]>([]);
  const [clients, setClients] = useState<Client[]>([]);
  const [images, setImages] = useState<ImageItem[]>([]);
  const [filterLabId, setFilterLabId] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const [addOpen, setAddOpen] = useState(false);
  const [bulkOpen, setBulkOpen] = useState(false);
  const [enrollOpen, setEnrollOpen] = useState(false);
  const [moveOpen, setMoveOpen] = useState(false);
  const [captureOpen, setCaptureOpen] = useState(false);
  const [deployOpen, setDeployOpen] = useState(false);

  const [labId, setLabId] = useState("");
  const [mac, setMac] = useState("");
  const [name, setName] = useState("");
  const [bulk, setBulk] = useState("");
  const [formError, setFormError] = useState("");

  const [enroll, setEnroll] = useState<EnrollmentSettings>({
    default_campus_id: "",
    default_lab_id: "",
  });
  const [moveLabId, setMoveLabId] = useState("");
  const [moveRename, setMoveRename] = useState(true);

  const [captureMac, setCaptureMac] = useState("");
  const [captureName, setCaptureName] = useState("");
  const [deployImage, setDeployImage] = useState("");
  const [diskPlan, setDiskPlan] = useState("wipe_whole_disk");
  const [deployTargets, setDeployTargets] = useState<Client[]>([]);

  const [editingId, setEditingId] = useState<string | null>(null);
  const [editName, setEditName] = useState("");
  const [editLabId, setEditLabId] = useState("");
  const [editNotes, setEditNotes] = useState("");

  const labMap = useMemo(
    () => new Map(labs.map((l) => [l.id, l])),
    [labs],
  );

  async function refresh() {
    const [l, camps, c, imgs] = await Promise.all([
      api.labs.list(),
      api.campuses.list(),
      api.clients.list(filterLabId || undefined),
      api.images.list(),
    ]);
    setLabs(l);
    setCampuses(camps);
    setClients(c);
    setImages(imgs);
    if (!labId && l[0]) setLabId(l[0].id);
    if (!moveLabId && l[0]) setMoveLabId(l[0].id);
    if (!deployImage && imgs[0]) setDeployImage(imgs[0].id);
    setSelected((prev) => {
      const ids = new Set(c.map((x) => x.id));
      return new Set([...prev].filter((id) => ids.has(id)));
    });
  }

  useEffect(() => {
    refresh().catch((err) => msg.error(err.message));
  }, [filterLabId]);

  function openAdd() {
    setMac("");
    setName("");
    setFormError("");
    if (!labId && labs[0]) setLabId(labs[0].id);
    setAddOpen(true);
  }

  function openBulk() {
    setBulk("");
    setFormError("");
    if (!labId && labs[0]) setLabId(labs[0].id);
    setBulkOpen(true);
  }

  async function openEnroll() {
    setFormError("");
    try {
      const e = await api.enrollment.get();
      setEnroll(e);
      setEnrollOpen(true);
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "读取登记设置失败");
    }
  }

  function openCapture() {
    if (selected.size === 0) {
      msg.warning("请先选择一台终端");
      return;
    }
    if (selected.size > 1) {
      msg.warning("采集系统一次只能选择一台终端");
      return;
    }
    const id = [...selected][0];
    const client = clients.find((c) => c.id === id);
    if (!client) {
      msg.warning("未找到选中的终端");
      return;
    }
    setCaptureMac(client.mac);
    setCaptureName(`${client.name}-金样`);
    setFormError("");
    setCaptureOpen(true);
  }

  function openDeploy() {
    if (selected.size === 0) {
      msg.warning("请先选择要清除缓存的终端");
      return;
    }
    const targets = clients.filter((c) => selected.has(c.id));
    if (targets.length === 0) {
      msg.warning("未找到选中的终端");
      return;
    }
    const resolved = targets.map((t) =>
      resolveDefaultImageId(t.lab_id, labs, campuses),
    );
    const unique = [...new Set(resolved.filter(Boolean))] as string[];
    if (unique.length === 1) {
      setDeployImage(unique[0]);
    } else if (!deployImage && images[0]) {
      setDeployImage(images[0].id);
    }
    setDeployTargets(targets);
    setDiskPlan("wipe_whole_disk");
    setFormError("");
    setDeployOpen(true);
  }

  async function createOne(e: FormEvent) {
    e.preventDefault();
    setFormError("");
    try {
      await api.clients.create({
        lab_id: labId,
        mac,
        name: name.trim() || undefined,
      });
      setAddOpen(false);
      msg.success("已添加终端");
      await refresh();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "添加失败");
    }
  }

  async function createBulk(e: FormEvent) {
    e.preventDefault();
    setFormError("");
    const macs = bulk
      .split(/[\n,;]+/)
      .map((s) => s.trim())
      .filter(Boolean);
    try {
      const res = await api.clients.bulk(labId, macs);
      setBulkOpen(false);
      msg.success(
        `批量完成：新增 ${res.created.length}，跳过 ${res.skipped.length}，错误 ${res.errors.length}`,
      );
      if (res.errors.length) msg.error(res.errors.join("; "));
      await refresh();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "批量失败");
    }
  }

  async function saveEnrollment(e: FormEvent) {
    e.preventDefault();
    setFormError("");
    try {
      const lab = labs.find((l) => l.id === enroll.default_lab_id);
      await api.enrollment.set({
        default_lab_id: enroll.default_lab_id,
        default_campus_id: lab?.campus_id || "",
      });
      setEnrollOpen(false);
      msg.success(
        enroll.default_lab_id
          ? "已保存自动登记默认机房"
          : "已清空默认机房（未知 MAC 不会自动入库）",
      );
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "保存失败");
    }
  }

  async function saveClient(id: string) {
    try {
      await api.clients.update(id, {
        lab_id: editLabId,
        name: editName,
        notes: editNotes,
      });
      setEditingId(null);
      await refresh();
      msg.success("已更新终端");
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "更新失败");
    }
  }

  async function removeClient(c: Client) {
    if (!window.confirm(`删除终端 ${c.mac}？`)) return;
    try {
      await api.clients.delete(c.id);
      if (editingId === c.id) setEditingId(null);
      await refresh();
      msg.success("已删除终端");
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "删除失败");
    }
  }

  function toggleAll(checked: boolean) {
    setSelected(checked ? new Set(clients.map((c) => c.id)) : new Set());
  }

  function toggleOne(id: string, checked: boolean) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (checked) next.add(id);
      else next.delete(id);
      return next;
    });
  }

  async function bulkDelete() {
    const ids = [...selected];
    if (!ids.length) return;
    if (!window.confirm(`批量删除选中的 ${ids.length} 台终端？`)) return;
    try {
      const res = await api.clients.bulkDelete(ids);
      msg.success(`已删除 ${res.affected} 台`);
      if (res.errors.length) msg.error(res.errors.join("; "));
      setSelected(new Set());
      await refresh();
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "批量删除失败");
    }
  }

  async function doBulkMove(e: FormEvent) {
    e.preventDefault();
    const ids = [...selected];
    if (!ids.length || !moveLabId) return;
    setFormError("");
    try {
      const res = await api.clients.bulkMove(ids, moveLabId, moveRename);
      setMoveOpen(false);
      msg.success(
        `已移动 ${res.affected} 台${moveRename ? "（并按目标机房规则重命名）" : ""}`,
      );
      if (res.errors.length) msg.error(res.errors.join("; "));
      setSelected(new Set());
      await refresh();
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "批量移动失败");
    }
  }

  async function onCapture(e: FormEvent) {
    e.preventDefault();
    setFormError("");
    try {
      await api.tasks.capture({
        mac: captureMac,
        name: captureName || undefined,
      });
      setCaptureOpen(false);
      msg.success("已创建采集任务");
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "创建失败");
    }
  }

  async function onDeployConfirm(e: FormEvent) {
    e.preventDefault();
    setFormError("");
    if (!deployImage) {
      setFormError("请选择镜像");
      return;
    }
    if (deployTargets.length === 0) {
      setFormError("没有目标终端");
      return;
    }
    try {
      await api.tasks.deploy({
        image_id: deployImage,
        macs: deployTargets.map((c) => c.mac),
        disk_plan: diskPlan,
        confirm_destructive: true,
      });
      setDeployOpen(false);
      setSelected(new Set());
      msg.success(`已创建部署任务（${deployTargets.length} 台）`);
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "创建失败");
    }
  }

  const allChecked =
    clients.length > 0 && clients.every((c) => selected.has(c.id));

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div className="panel-head">
        <div>
          <h1>终端</h1>
          <p className="muted">
            Agent 心跳可自动登记到默认机房；支持按机房筛选与批量操作。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn ghost" onClick={openEnroll}>
            自动登记设置
          </button>
          <button
            type="button"
            className="btn"
            onClick={openAdd}
            disabled={labs.length === 0}
          >
            添加终端
          </button>
          <button
            type="button"
            className="btn accent"
            onClick={openBulk}
            disabled={labs.length === 0}
          >
            批量导入
          </button>
        </div>
      </div>
      {labs.length === 0 ? (
        <p className="muted">请先在「校区 / 机房」中创建机房，再登记终端。</p>
      ) : null}

      <div className="card">
        <div className="panel-head">
          <div className="page-actions">
            <label className="muted">机房筛选</label>
            <select
              value={filterLabId}
              onChange={(e) => setFilterLabId(e.target.value)}
            >
              <option value="">全部机房</option>
              {labs.map((lab) => (
                <option key={lab.id} value={lab.id}>
                  {lab.name}
                </option>
              ))}
            </select>
            <span className="muted">{clients.length} 台</span>
          </div>
          <div className="page-actions">
            <button type="button" className="btn" onClick={openCapture}>
              采集系统
            </button>
            <button type="button" className="btn accent" onClick={openDeploy}>
              清除缓存
            </button>
            <button
              type="button"
              className="btn ghost"
              disabled={selected.size === 0}
              onClick={() => {
                setMoveLabId(filterLabId || labs[0]?.id || "");
                setFormError("");
                setMoveOpen(true);
              }}
            >
              批量移动 ({selected.size})
            </button>
            <button
              type="button"
              className="btn danger"
              disabled={selected.size === 0}
              onClick={bulkDelete}
            >
              批量删除
            </button>
          </div>
        </div>
        <table>
          <thead>
            <tr>
              <th>
                <input
                  type="checkbox"
                  checked={allChecked}
                  onChange={(e) => toggleAll(e.target.checked)}
                  aria-label="全选"
                />
              </th>
              <th>MAC</th>
              <th>名称</th>
              <th>机房</th>
              <th>最近上报</th>
              <th>备注</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {clients.length === 0 ? (
              <tr>
                <td colSpan={7} className="muted">
                  暂无终端。可手动添加，或配置默认机房后由 Agent 自动上报入库。
                </td>
              </tr>
            ) : (
              clients.map((c) => (
                <tr key={c.id}>
                  <td>
                    <input
                      type="checkbox"
                      checked={selected.has(c.id)}
                      onChange={(e) => toggleOne(c.id, e.target.checked)}
                      aria-label={`选择 ${c.mac}`}
                    />
                  </td>
                  <td className="mono">{c.mac}</td>
                  <td>
                    {editingId === c.id ? (
                      <input
                        value={editName}
                        onChange={(e) => setEditName(e.target.value)}
                      />
                    ) : (
                      c.name || "—"
                    )}
                  </td>
                  <td>
                    {editingId === c.id ? (
                      <select
                        value={editLabId}
                        onChange={(e) => setEditLabId(e.target.value)}
                      >
                        {labs.map((lab) => (
                          <option key={lab.id} value={lab.id}>
                            {lab.name}
                          </option>
                        ))}
                      </select>
                    ) : (
                      labMap.get(c.lab_id)?.name || c.lab_id
                    )}
                  </td>
                  <td className="muted">
                    {c.last_seen
                      ? new Date(c.last_seen).toLocaleString()
                      : "—"}
                  </td>
                  <td>
                    {editingId === c.id ? (
                      <input
                        value={editNotes}
                        onChange={(e) => setEditNotes(e.target.value)}
                      />
                    ) : (
                      c.notes || "—"
                    )}
                  </td>
                  <td>
                    <div className="row-actions">
                      {editingId === c.id ? (
                        <>
                          <button
                            type="button"
                            className="btn"
                            onClick={() => saveClient(c.id)}
                          >
                            保存
                          </button>
                          <button
                            type="button"
                            className="btn ghost"
                            onClick={() => setEditingId(null)}
                          >
                            取消
                          </button>
                        </>
                      ) : (
                        <>
                          <button
                            type="button"
                            className="btn ghost"
                            onClick={() => {
                              setEditingId(c.id);
                              setEditName(c.name || "");
                              setEditLabId(c.lab_id);
                              setEditNotes(c.notes || "");
                            }}
                          >
                            编辑
                          </button>
                          <button
                            type="button"
                            className="btn danger"
                            onClick={() => removeClient(c)}
                          >
                            删除
                          </button>
                        </>
                      )}
                    </div>
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      <Modal open={addOpen} title="添加终端" onClose={() => setAddOpen(false)}>
        <form onSubmit={createOne}>
          {formError ? <p className="error">{formError}</p> : null}
          <div className="field">
            <label>机房</label>
            <select
              value={labId}
              onChange={(e) => setLabId(e.target.value)}
              required
            >
              {labs.map((lab) => (
                <option key={lab.id} value={lab.id}>
                  {lab.name}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>MAC</label>
            <input
              className="mono"
              value={mac}
              onChange={(e) => setMac(e.target.value)}
              placeholder="AA:BB:CC:DD:EE:FF"
              autoFocus
              required
            />
          </div>
          <div className="field">
            <label>名称（留空则按机房规则自动生成）</label>
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </div>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setAddOpen(false)}
            >
              取消
            </button>
            <button className="btn" type="submit" disabled={!labId}>
              添加
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        open={bulkOpen}
        title="批量导入 MAC"
        onClose={() => setBulkOpen(false)}
      >
        <form onSubmit={createBulk}>
          {formError ? <p className="error">{formError}</p> : null}
          <div className="field">
            <label>机房</label>
            <select
              value={labId}
              onChange={(e) => setLabId(e.target.value)}
              required
            >
              {labs.map((lab) => (
                <option key={lab.id} value={lab.id}>
                  {lab.name}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>每行一个 MAC（名称按机房规则自动生成）</label>
            <textarea
              className="mono"
              rows={8}
              value={bulk}
              onChange={(e) => setBulk(e.target.value)}
              autoFocus
              required
            />
          </div>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setBulkOpen(false)}
            >
              取消
            </button>
            <button className="btn accent" type="submit" disabled={!labId}>
              导入
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        open={enrollOpen}
        title="自动登记设置"
        onClose={() => setEnrollOpen(false)}
      >
        <form onSubmit={saveEnrollment}>
          {formError ? <p className="error">{formError}</p> : null}
          <p className="muted">
            Agent 上报未知 MAC 时，自动登记到下方默认机房，并按该机房命名规则生成名称（如
            stu001）。
          </p>
          <div className="field">
            <label>默认机房</label>
            <select
              value={enroll.default_lab_id}
              onChange={(e) =>
                setEnroll({
                  ...enroll,
                  default_lab_id: e.target.value,
                })
              }
            >
              <option value="">不自动登记</option>
              {labs.map((lab) => (
                <option key={lab.id} value={lab.id}>
                  {lab.name}
                </option>
              ))}
            </select>
          </div>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setEnrollOpen(false)}
            >
              取消
            </button>
            <button className="btn" type="submit">
              保存
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        open={moveOpen}
        title={`批量移动（${selected.size} 台）`}
        onClose={() => setMoveOpen(false)}
      >
        <form onSubmit={doBulkMove}>
          {formError ? <p className="error">{formError}</p> : null}
          <div className="field">
            <label>目标机房</label>
            <select
              value={moveLabId}
              onChange={(e) => setMoveLabId(e.target.value)}
              required
            >
              {labs.map((lab) => (
                <option key={lab.id} value={lab.id}>
                  {lab.name}
                </option>
              ))}
            </select>
          </div>
          <label className="muted" style={{ display: "flex", gap: "0.5rem" }}>
            <input
              type="checkbox"
              checked={moveRename}
              onChange={(e) => setMoveRename(e.target.checked)}
            />
            按目标机房命名规则重新命名
          </label>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setMoveOpen(false)}
            >
              取消
            </button>
            <button className="btn" type="submit" disabled={!moveLabId}>
              移动
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        open={captureOpen}
        title="采集系统"
        onClose={() => setCaptureOpen(false)}
      >
        <form onSubmit={onCapture}>
          {formError ? <p className="error">{formError}</p> : null}
          <p className="muted">
            将对终端 <span className="mono">{captureMac}</span> 下发采集任务。
          </p>
          <div className="field">
            <label>任务/镜像名</label>
            <input
              value={captureName}
              onChange={(e) => setCaptureName(e.target.value)}
              placeholder="机房A-金样"
            />
          </div>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setCaptureOpen(false)}
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
        open={deployOpen}
        title="清除缓存"
        onClose={() => setDeployOpen(false)}
      >
        <form onSubmit={onDeployConfirm}>
          {formError ? <p className="error">{formError}</p> : null}
          <p className="error">
            将对以下 {deployTargets.length}{" "}
            台终端下发部署任务（覆盖磁盘），请确认后继续。
          </p>
          <ul className="muted" style={{ margin: "0 0 0.75rem", paddingLeft: "1.2rem" }}>
            {deployTargets.map((c) => (
              <li key={c.id}>
                {c.name}（<span className="mono">{c.mac}</span>）
              </li>
            ))}
          </ul>
          <div className="field">
            <label>镜像</label>
            <select
              value={deployImage}
              onChange={(e) => setDeployImage(e.target.value)}
              required
            >
              {images.length === 0 ? (
                <option value="">暂无镜像，请先在镜像库登记</option>
              ) : null}
              {images.map((img) => (
                <option key={img.id} value={img.id}>
                  {img.name}
                </option>
              ))}
            </select>
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
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setDeployOpen(false)}
            >
              取消
            </button>
            <button
              className="btn danger"
              type="submit"
              disabled={!deployImage || deployTargets.length === 0}
            >
              确认清除并部署
            </button>
          </div>
        </form>
      </Modal>
    </div>
  );
}
