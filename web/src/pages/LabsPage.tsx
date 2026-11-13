import { FormEvent, useEffect, useState } from "react";
import { api, type Campus, type Lab } from "../api";
import { Modal } from "../components/Modal";

function previewName(prefix: string, width: number, start: number) {
  const w = Math.min(8, Math.max(1, width || 3));
  const n = Math.max(1, start || 1);
  return `${prefix || ""}${String(n).padStart(w, "0")}`;
}

export function LabsPage() {
  const [campuses, setCampuses] = useState<Campus[]>([]);
  const [labs, setLabs] = useState<Lab[]>([]);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");

  const [campusModalOpen, setCampusModalOpen] = useState(false);
  const [campusName, setCampusName] = useState("");
  const [campusFormError, setCampusFormError] = useState("");

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

  const [editingCampusId, setEditingCampusId] = useState<string | null>(null);
  const [editCampusName, setEditCampusName] = useState("");

  async function refresh() {
    const [c, l] = await Promise.all([api.campuses.list(), api.labs.list()]);
    setCampuses(c);
    setLabs(l);
  }

  useEffect(() => {
    refresh().catch((err) => setError(err.message));
  }, []);

  function openCampusModal() {
    setCampusName("");
    setCampusFormError("");
    setCampusModalOpen(true);
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
    setSettingsError("");
  }

  async function createCampus(e: FormEvent) {
    e.preventDefault();
    setCampusFormError("");
    try {
      await api.campuses.create(campusName);
      setCampusModalOpen(false);
      setMessage("已创建校区");
      await refresh();
    } catch (err) {
      setCampusFormError(err instanceof Error ? err.message : "创建校区失败");
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
      setMessage(`已在「${labModalCampus.name}」下创建机房`);
      await refresh();
    } catch (err) {
      setLabFormError(err instanceof Error ? err.message : "创建机房失败");
    }
  }

  async function saveCampus(id: string) {
    setError("");
    try {
      await api.campuses.update(id, editCampusName);
      setEditingCampusId(null);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "更新校区失败");
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
    setError("");
    try {
      await api.campuses.delete(c.id);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "删除校区失败");
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
      });
      setSettingsLab(null);
      setMessage("已保存机房设置");
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
    setError("");
    try {
      await api.labs.delete(lab.id);
      if (settingsLab?.id === lab.id) setSettingsLab(null);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "删除机房失败");
    }
  }

  return (
    <div className="grid" style={{ gap: "1.25rem" }}>
      <div className="panel-head">
        <div>
          <h1>校区 / 机房</h1>
          <p className="muted">
            机房可配置命名规则与 IP 段；终端自动登记时按规则生成名称。
          </p>
        </div>
        <div className="page-actions">
          <button type="button" className="btn" onClick={openCampusModal}>
            添加校区
          </button>
        </div>
      </div>
      {error ? <p className="error">{error}</p> : null}
      {message ? <p className="muted">{message}</p> : null}

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
              <th>ID</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {campuses.length === 0 ? (
              <tr>
                <td colSpan={4} className="muted">
                  暂无校区，点击右上角「添加校区」开始。
                </td>
              </tr>
            ) : (
              campuses.map((c) => (
                <tr key={c.id}>
                  <td>
                    {editingCampusId === c.id ? (
                      <input
                        value={editCampusName}
                        onChange={(e) => setEditCampusName(e.target.value)}
                      />
                    ) : (
                      c.name
                    )}
                  </td>
                  <td className="muted">
                    {labs.filter((l) => l.campus_id === c.id).length}
                  </td>
                  <td className="mono muted">{c.id.slice(0, 8)}</td>
                  <td>
                    <div className="row-actions">
                      {editingCampusId === c.id ? (
                        <>
                          <button
                            type="button"
                            className="btn"
                            onClick={() => saveCampus(c.id)}
                          >
                            保存
                          </button>
                          <button
                            type="button"
                            className="btn ghost"
                            onClick={() => setEditingCampusId(null)}
                          >
                            取消
                          </button>
                        </>
                      ) : (
                        <>
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
                            onClick={() => {
                              setEditingCampusId(c.id);
                              setEditCampusName(c.name);
                            }}
                          >
                            编辑
                          </button>
                          <button
                            type="button"
                            className="btn danger"
                            onClick={() => removeCampus(c)}
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
              <th>IP 段</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {labs.length === 0 ? (
              <tr>
                <td colSpan={5} className="muted">
                  暂无机房。在上方校区行点击「增加机房」。
                </td>
              </tr>
            ) : (
              labs.map((lab) => (
                <tr key={lab.id}>
                  <td>{lab.name}</td>
                  <td>
                    {campuses.find((c) => c.id === lab.campus_id)?.name ||
                      lab.campus_id}
                  </td>
                  <td className="mono muted">
                    {previewName(
                      lab.name_prefix || "",
                      lab.suffix_width ?? 3,
                      lab.next_suffix ?? lab.suffix_start ?? 1,
                    )}
                  </td>
                  <td className="mono muted">
                    {lab.ip_start || lab.ip_end
                      ? `${lab.ip_start || "?"} – ${lab.ip_end || "?"}`
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
              ))
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
