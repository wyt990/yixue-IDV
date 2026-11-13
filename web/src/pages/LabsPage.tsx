import { FormEvent, useEffect, useState } from "react";
import {
  api,
  type Campus,
  type EdgeNode,
  type ImageItem,
  type Lab,
} from "../api";
import { Modal } from "../components/Modal";
import { useMessage } from "../message";

function previewName(prefix: string, width: number, start: number) {
  const w = Math.min(8, Math.max(1, width || 3));
  const n = Math.max(1, start || 1);
  return `${prefix || ""}${String(n).padStart(w, "0")}`;
}

function imageLabel(images: ImageItem[], id?: string | null) {
  if (!id) return "—";
  return images.find((i) => i.id === id)?.name || id.slice(0, 8);
}

function EdgeChecklist({
  edges,
  selected,
  onChange,
}: {
  edges: EdgeNode[];
  selected: string[];
  onChange: (ids: string[]) => void;
}) {
  function toggle(id: string, on: boolean) {
    if (on) {
      if (selected.includes(id)) return;
      onChange([...selected, id]);
    } else {
      onChange(selected.filter((x) => x !== id));
    }
  }

  function move(id: string, dir: -1 | 1) {
    const i = selected.indexOf(id);
    if (i < 0) return;
    const j = i + dir;
    if (j < 0 || j >= selected.length) return;
    const next = [...selected];
    [next[i], next[j]] = [next[j], next[i]];
    onChange(next);
  }

  return (
    <div>
      {edges.length === 0 ? (
        <p className="muted">暂无边缘节点，请先在「边缘」登记。</p>
      ) : (
        <div style={{ display: "grid", gap: "0.35rem" }}>
          {edges.map((ed) => {
            const checked = selected.includes(ed.id);
            const order = selected.indexOf(ed.id);
            return (
              <div
                key={ed.id}
                className="page-actions"
                style={{ justifyContent: "space-between" }}
              >
                <label
                  className="muted"
                  style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={(e) => toggle(ed.id, e.target.checked)}
                  />
                  {ed.name}
                  {checked ? (
                    <span className="mono">优先 {order + 1}</span>
                  ) : null}
                </label>
                {checked ? (
                  <span className="page-actions">
                    <button
                      type="button"
                      className="btn ghost"
                      onClick={() => move(ed.id, -1)}
                      disabled={order <= 0}
                    >
                      上移
                    </button>
                    <button
                      type="button"
                      className="btn ghost"
                      onClick={() => move(ed.id, 1)}
                      disabled={order < 0 || order >= selected.length - 1}
                    >
                      下移
                    </button>
                  </span>
                ) : null}
              </div>
            );
          })}
        </div>
      )}
      <p className="muted" style={{ marginTop: "0.5rem" }}>
        勾选顺序可调；未绑定时回落校区边缘池，再否则任务可选「任意」。
      </p>
    </div>
  );
}

export function LabsPage() {
  const msg = useMessage();
  const [campuses, setCampuses] = useState<Campus[]>([]);
  const [labs, setLabs] = useState<Lab[]>([]);
  const [images, setImages] = useState<ImageItem[]>([]);
  const [edges, setEdges] = useState<EdgeNode[]>([]);

  const [campusModalOpen, setCampusModalOpen] = useState(false);
  const [campusName, setCampusName] = useState("");
  const [campusFormError, setCampusFormError] = useState("");

  const [campusSettings, setCampusSettings] = useState<Campus | null>(null);
  const [csName, setCsName] = useState("");
  const [csImageId, setCsImageId] = useState("");
  const [csEdgeIds, setCsEdgeIds] = useState<string[]>([]);
  const [csError, setCsError] = useState("");

  const [labModalCampus, setLabModalCampus] = useState<Campus | null>(null);
  const [labName, setLabName] = useState("");
  const [labPrefix, setLabPrefix] = useState("stu");
  const [labWidth, setLabWidth] = useState(3);
  const [labStart, setLabStart] = useState(1);
  const [labIpStart, setLabIpStart] = useState("");
  const [labIpEnd, setLabIpEnd] = useState("");
  const [labFormError, setLabFormError] = useState("");

  const [settingsLab, setSettingsLab] = useState<Lab | null>(null);
  const [settingsError, setSettingsError] = useState("");
  const [editName, setEditName] = useState("");
  const [editCampusId, setEditCampusId] = useState("");
  const [editPrefix, setEditPrefix] = useState("");
  const [editWidth, setEditWidth] = useState(3);
  const [editStart, setEditStart] = useState(1);
  const [editNext, setEditNext] = useState(1);
  const [editIpStart, setEditIpStart] = useState("");
  const [editIpEnd, setEditIpEnd] = useState("");
  const [editImageId, setEditImageId] = useState("");
  const [editEdgeIds, setEditEdgeIds] = useState<string[]>([]);

  async function refresh() {
    const [c, l, imgs, eds] = await Promise.all([
      api.campuses.list(),
      api.labs.list(),
      api.images.list(),
      api.edges.list(),
    ]);
    setCampuses(c);
    setLabs(l);
    setImages(imgs);
    setEdges(eds);
  }

  useEffect(() => {
    refresh().catch((err) => msg.error(err.message));
  }, []);

  function openCampusModal() {
    setCampusName("");
    setCampusFormError("");
    setCampusModalOpen(true);
  }

  function openCampusSettings(c: Campus) {
    setCampusSettings(c);
    setCsName(c.name);
    setCsImageId(c.default_image_id || "");
    setCsEdgeIds([...(c.edge_ids || [])]);
    setCsError("");
  }

  function openLabModal(campus: Campus) {
    setLabModalCampus(campus);
    setLabName("");
    setLabPrefix("stu");
    setLabWidth(3);
    setLabStart(1);
    setLabIpStart("");
    setLabIpEnd("");
    setLabFormError("");
  }

  function openLabSettings(lab: Lab) {
    setSettingsLab(lab);
    setEditName(lab.name);
    setEditCampusId(lab.campus_id);
    setEditPrefix(lab.name_prefix || "");
    setEditWidth(lab.suffix_width ?? 3);
    setEditStart(lab.suffix_start ?? 1);
    setEditNext(lab.next_suffix ?? lab.suffix_start ?? 1);
    setEditIpStart(lab.ip_start || "");
    setEditIpEnd(lab.ip_end || "");
    setEditImageId(lab.default_image_id || "");
    setEditEdgeIds([...(lab.edge_ids || [])]);
    setSettingsError("");
  }

  async function createCampus(e: FormEvent) {
    e.preventDefault();
    setCampusFormError("");
    try {
      await api.campuses.create(campusName);
      setCampusModalOpen(false);
      msg.success("已创建校区");
      await refresh();
    } catch (err) {
      setCampusFormError(err instanceof Error ? err.message : "创建校区失败");
    }
  }

  async function saveCampusSettings(e: FormEvent) {
    e.preventDefault();
    if (!campusSettings) return;
    setCsError("");
    try {
      await api.campuses.update(campusSettings.id, {
        name: csName,
        default_image_id: csImageId || null,
        edge_ids: csEdgeIds,
      });
      setCampusSettings(null);
      msg.success("已保存校区设置");
      await refresh();
    } catch (err) {
      setCsError(err instanceof Error ? err.message : "保存失败");
    }
  }

  async function createLab(e: FormEvent) {
    e.preventDefault();
    if (!labModalCampus) return;
    setLabFormError("");
    try {
      await api.labs.create(labModalCampus.id, labName, {
        name_prefix: labPrefix,
        suffix_width: labWidth,
        suffix_start: labStart,
        ip_start: labIpStart,
        ip_end: labIpEnd,
      });
      setLabModalCampus(null);
      msg.success(`已在「${labModalCampus.name}」下创建机房`);
      await refresh();
    } catch (err) {
      setLabFormError(err instanceof Error ? err.message : "创建机房失败");
    }
  }

  async function removeCampus(c: Campus) {
    if (
      !window.confirm(
        `删除校区「${c.name}」？其下机房与终端也会一并删除。`,
      )
    ) {
      return;
    }
    try {
      await api.campuses.delete(c.id);
      msg.success("已删除校区");
      await refresh();
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "删除校区失败");
    }
  }

  async function saveLabSettings(e: FormEvent) {
    e.preventDefault();
    if (!settingsLab) return;
    setSettingsError("");
    try {
      await api.labs.update(settingsLab.id, {
        name: editName,
        campus_id: editCampusId,
        name_prefix: editPrefix,
        suffix_width: editWidth,
        suffix_start: editStart,
        next_suffix: editNext,
        ip_start: editIpStart,
        ip_end: editIpEnd,
        default_image_id: editImageId || null,
        edge_ids: editEdgeIds,
      });
      setSettingsLab(null);
      msg.success("已保存机房设置");
      await refresh();
    } catch (err) {
      setSettingsError(err instanceof Error ? err.message : "保存失败");
    }
  }

  async function removeLab(lab: Lab) {
    if (
      !window.confirm(
        `删除机房「${lab.name}」？其下终端也会一并删除。`,
      )
    ) {
      return;
    }
    try {
      await api.labs.delete(lab.id);
      if (settingsLab?.id === lab.id) setSettingsLab(null);
      msg.success("已删除机房");
      await refresh();
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "删除机房失败");
    }
  }

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div className="panel-head">
        <div>
          <h1>校区 / 机房</h1>
          <p className="muted">
            可配置命名规则、默认基线镜像与边缘绑定；机房未配时回落校区。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={openCampusModal}>
            添加校区
          </button>
        </div>
      </div>

      <div className="card">
        <div className="panel-head">
          <h2>校区列表</h2>
          <span className="muted">{campuses.length} 个</span>
        </div>
        <table>
          <thead>
            <tr>
              <th>名称</th>
              <th>机房数</th>
              <th>默认镜像</th>
              <th>边缘</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {campuses.length === 0 ? (
              <tr>
                <td colSpan={5} className="muted">
                  暂无校区，点击右上角「添加校区」开始。
                </td>
              </tr>
            ) : (
              campuses.map((c) => (
                <tr key={c.id}>
                  <td>{c.name}</td>
                  <td className="muted">
                    {labs.filter((l) => l.campus_id === c.id).length}
                  </td>
                  <td className="muted">
                    {imageLabel(images, c.default_image_id)}
                  </td>
                  <td className="muted">
                    {(c.edge_ids || []).length
                      ? `${(c.edge_ids || []).length} 个`
                      : "—"}
                  </td>
                  <td>
                    <div className="row-actions">
                      <button
                        type="button"
                        className="btn accent"
                        onClick={() => openLabModal(c)}
                      >
                        增加机房
                      </button>
                      <button
                        type="button"
                        className="btn ghost"
                        onClick={() => openCampusSettings(c)}
                      >
                        设置
                      </button>
                      <button
                        type="button"
                        className="btn danger"
                        onClick={() => removeCampus(c)}
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
        <div className="panel-head">
          <h2>机房列表</h2>
          <span className="muted">{labs.length} 个</span>
        </div>
        <table>
          <thead>
            <tr>
              <th>机房</th>
              <th>校区</th>
              <th>命名预览</th>
              <th>默认镜像</th>
              <th>边缘</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {labs.length === 0 ? (
              <tr>
                <td colSpan={6} className="muted">
                  暂无机房。在上方校区行点击「增加机房」。
                </td>
              </tr>
            ) : (
              labs.map((lab) => {
                const campus = campuses.find((c) => c.id === lab.campus_id);
                const effectiveImg =
                  lab.default_image_id || campus?.default_image_id || null;
                return (
                  <tr key={lab.id}>
                    <td>{lab.name}</td>
                    <td>{campus?.name || lab.campus_id}</td>
                    <td className="mono muted">
                      {previewName(
                        lab.name_prefix || "",
                        lab.suffix_width ?? 3,
                        lab.next_suffix ?? lab.suffix_start ?? 1,
                      )}
                    </td>
                    <td className="muted">
                      {imageLabel(images, effectiveImg)}
                      {!lab.default_image_id && campus?.default_image_id ? (
                        <span className="muted">（校区）</span>
                      ) : null}
                    </td>
                    <td className="muted">
                      {(lab.edge_ids || []).length
                        ? `${(lab.edge_ids || []).length} 个`
                        : (campus?.edge_ids || []).length
                          ? `${(campus?.edge_ids || []).length} 个（校区）`
                          : "—"}
                    </td>
                    <td>
                      <div className="row-actions">
                        <button
                          type="button"
                          className="btn ghost"
                          onClick={() => openLabSettings(lab)}
                        >
                          设置
                        </button>
                        <button
                          type="button"
                          className="btn danger"
                          onClick={() => removeLab(lab)}
                        >
                          删除
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>

      <Modal
        open={campusModalOpen}
        title="添加校区"
        onClose={() => setCampusModalOpen(false)}
      >
        <form onSubmit={createCampus}>
          {campusFormError ? <p className="error">{campusFormError}</p> : null}
          <div className="field">
            <label>名称</label>
            <input
              value={campusName}
              onChange={(e) => setCampusName(e.target.value)}
              autoFocus
              required
            />
          </div>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setCampusModalOpen(false)}
            >
              取消
            </button>
            <button className="btn" type="submit">
              创建
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        open={!!campusSettings}
        title={
          campusSettings ? `校区设置 · ${campusSettings.name}` : "校区设置"
        }
        onClose={() => setCampusSettings(null)}
        wide
      >
        <form onSubmit={saveCampusSettings}>
          {csError ? <p className="error">{csError}</p> : null}
          <div className="field">
            <label>名称</label>
            <input
              value={csName}
              onChange={(e) => setCsName(e.target.value)}
              required
            />
          </div>
          <div className="field">
            <label>默认基线镜像（机房未配时回落）</label>
            <select
              value={csImageId}
              onChange={(e) => setCsImageId(e.target.value)}
            >
              <option value="">不设置</option>
              {images.map((img) => (
                <option key={img.id} value={img.id}>
                  {img.name}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>绑定边缘（可多选）</label>
            <EdgeChecklist
              edges={edges}
              selected={csEdgeIds}
              onChange={setCsEdgeIds}
            />
          </div>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setCampusSettings(null)}
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
        open={!!labModalCampus}
        title={
          labModalCampus
            ? `增加机房 · ${labModalCampus.name}`
            : "增加机房"
        }
        onClose={() => setLabModalCampus(null)}
        wide
      >
        <form onSubmit={createLab}>
          {labFormError ? <p className="error">{labFormError}</p> : null}
          <div className="field">
            <label>机房名称</label>
            <input
              value={labName}
              onChange={(e) => setLabName(e.target.value)}
              autoFocus
              required
            />
          </div>
          <div className="form-row">
            <div className="field">
              <label>终端名前缀</label>
              <input
                value={labPrefix}
                onChange={(e) => setLabPrefix(e.target.value)}
                placeholder="stu"
              />
            </div>
            <div className="field">
              <label>后缀位数</label>
              <input
                type="number"
                min={1}
                max={8}
                value={labWidth}
                onChange={(e) => setLabWidth(Number(e.target.value) || 3)}
              />
            </div>
          </div>
          <div className="form-row">
            <div className="field">
              <label>起始序号</label>
              <input
                type="number"
                min={1}
                value={labStart}
                onChange={(e) => setLabStart(Number(e.target.value) || 1)}
              />
            </div>
            <div className="field">
              <label>预览</label>
              <input
                className="mono"
                value={previewName(labPrefix, labWidth, labStart)}
                readOnly
              />
            </div>
          </div>
          <div className="form-row">
            <div className="field">
              <label>IP 起始</label>
              <input
                className="mono"
                value={labIpStart}
                onChange={(e) => setLabIpStart(e.target.value)}
                placeholder="192.168.1.1"
              />
            </div>
            <div className="field">
              <label>IP 结束</label>
              <input
                className="mono"
                value={labIpEnd}
                onChange={(e) => setLabIpEnd(e.target.value)}
                placeholder="192.168.1.254"
              />
            </div>
          </div>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setLabModalCampus(null)}
            >
              取消
            </button>
            <button className="btn accent" type="submit">
              创建机房
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        open={!!settingsLab}
        title={settingsLab ? `机房设置 · ${settingsLab.name}` : "机房设置"}
        onClose={() => setSettingsLab(null)}
        wide
      >
        <form onSubmit={saveLabSettings}>
          {settingsError ? <p className="error">{settingsError}</p> : null}
          <div className="form-row">
            <div className="field">
              <label>机房名称</label>
              <input
                value={editName}
                onChange={(e) => setEditName(e.target.value)}
                required
              />
            </div>
            <div className="field">
              <label>所属校区</label>
              <select
                value={editCampusId}
                onChange={(e) => setEditCampusId(e.target.value)}
              >
                {campuses.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <div className="form-row">
            <div className="field">
              <label>终端名前缀</label>
              <input
                value={editPrefix}
                onChange={(e) => setEditPrefix(e.target.value)}
              />
            </div>
            <div className="field">
              <label>后缀位数</label>
              <input
                type="number"
                min={1}
                max={8}
                value={editWidth}
                onChange={(e) => setEditWidth(Number(e.target.value) || 3)}
              />
            </div>
          </div>
          <div className="form-row">
            <div className="field">
              <label>起始序号</label>
              <input
                type="number"
                min={1}
                value={editStart}
                onChange={(e) => setEditStart(Number(e.target.value) || 1)}
              />
            </div>
            <div className="field">
              <label>下一个序号</label>
              <input
                type="number"
                min={1}
                value={editNext}
                onChange={(e) => setEditNext(Number(e.target.value) || 1)}
              />
            </div>
          </div>
          <p className="muted">
            下一台自动命名预览：
            <span className="mono">
              {" "}
              {previewName(editPrefix, editWidth, editNext)}
            </span>
          </p>
          <div className="form-row">
            <div className="field">
              <label>IP 起始</label>
              <input
                className="mono"
                value={editIpStart}
                onChange={(e) => setEditIpStart(e.target.value)}
              />
            </div>
            <div className="field">
              <label>IP 结束</label>
              <input
                className="mono"
                value={editIpEnd}
                onChange={(e) => setEditIpEnd(e.target.value)}
              />
            </div>
          </div>
          <div className="field">
            <label>默认基线镜像</label>
            <select
              value={editImageId}
              onChange={(e) => setEditImageId(e.target.value)}
            >
              <option value="">不设置（回落校区）</option>
              {images.map((img) => (
                <option key={img.id} value={img.id}>
                  {img.name}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>绑定边缘（可多选）</label>
            <EdgeChecklist
              edges={edges}
              selected={editEdgeIds}
              onChange={setEditEdgeIds}
            />
          </div>
          <div className="modal-actions">
            <button
              type="button"
              className="btn ghost"
              onClick={() => setSettingsLab(null)}
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
