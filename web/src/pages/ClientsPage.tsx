import { FormEvent, useEffect, useMemo, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import {
  api,
  type Campus,
  type Client,
  type CommandPreset,
  type EnrollmentSettings,
  type ImageItem,
  type Lab,
} from "../api";
import { AssistStage } from "../components/AssistStage";
import { Drawer } from "../components/Drawer";
import { Modal } from "../components/Modal";
import { PageHeader } from "../components/PageHeader";
import { readLabId, writeLabId } from "../labScope";
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
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const [labs, setLabs] = useState<Lab[]>([]);
  const [campuses, setCampuses] = useState<Campus[]>([]);
  const [clients, setClients] = useState<Client[]>([]);
  const [images, setImages] = useState<ImageItem[]>([]);
  const [filterLabId, setFilterLabId] = useState(() => {
    const id = params.get("lab") || readLabId();
    writeLabId(id);
    return id;
  });
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [detail, setDetail] = useState<Client | null>(null);
  const [stage, setStage] = useState<{ mac: string; mode: "view" | "control" } | null>(
    null,
  );
  const [moreOpen, setMoreOpen] = useState(false);
  const [powerOpen, setPowerOpen] = useState(false);
  const [powerRows, setPowerRows] = useState<
    { id: string; mac: string; kind: string; state: string; detail: string }[]
  >([]);
  const [fileKind, setFileKind] = useState<null | "collect" | "distribute">(null);
  const [collectDir, setCollectDir] = useState("C:\\Users\\Public\\Documents");
  const [targetDir, setTargetDir] = useState("C:\\Users\\Public\\Documents");
  const [batchFile, setBatchFile] = useState<File | null>(null);
  const [commandMenuOpen, setCommandMenuOpen] = useState(false);
  const [presetKind, setPresetKind] = useState<null | "command" | "url" | "program">(null);
  const [presets, setPresets] = useState<CommandPreset[]>([]);
  const [presetEditing, setPresetEditing] = useState(false);
  const [presetId, setPresetId] = useState<string | null>(null);
  const [presetName, setPresetName] = useState("");
  const [presetCommand, setPresetCommand] = useState("");
  const [presetUrl, setPresetUrl] = useState("https://");
  const [presetProgram, setPresetProgram] = useState("");
  const [presetArgs, setPresetArgs] = useState("");
  const [commandError, setCommandError] = useState("");

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
  const [excludeTemp, setExcludeTemp] = useState(true);
  const [deployImage, setDeployImage] = useState("");
  const [diskPlan, setDiskPlan] = useState("wipe_whole_disk");
  const [deployTargets, setDeployTargets] = useState<Client[]>([]);

  const [editName, setEditName] = useState("");
  const [editLabId, setEditLabId] = useState("");
  const [editNotes, setEditNotes] = useState("");
  const [editAssignedIp, setEditAssignedIp] = useState("");

  const labMap = useMemo(
    () => new Map(labs.map((l) => [l.id, l])),
    [labs],
  );
  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return clients.filter((c) => {
      if (status === "online" && !c.online) return false;
      if (status === "offline" && (!c.agent_last_seen || c.online)) return false;
      if (status === "unfrozen" && (!c.agent_last_seen || c.frozen)) return false;
      if (!q) return true;
      return [c.name, c.mac, c.last_ip, c.assigned_ip, c.notes]
        .filter(Boolean)
        .join(" ")
        .toLowerCase()
        .includes(q);
    });
  }, [clients, query, status]);
  const picked = useMemo(
    () => clients.filter((c) => selected.has(c.id)),
    [clients, selected],
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

  useEffect(() => {
    if (!moreOpen) return;
    function close() {
      setMoreOpen(false);
    }
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [moreOpen]);

  useEffect(() => {
    const mac = params.get("assist");
    if (!mac) return;
    const mode = params.get("mode") === "control" ? "control" : "view";
    setStage({ mac, mode });
    const next = new URLSearchParams(params);
    next.delete("assist");
    next.delete("mode");
    setParams(next, { replace: true });
  }, []);

  useEffect(() => {
    setDetail((cur) => {
      if (!cur) return cur;
      return clients.find((c) => c.id === cur.id) ?? null;
    });
  }, [clients]);

  // Live status: online/offline + restore state refresh (agent heartbeat ~5s).
  useEffect(() => {
    const timer = window.setInterval(() => {
      refresh().catch(() => {});
    }, 5000);
    return () => window.clearInterval(timer);
  }, [filterLabId]);

  function onlineText(c: Client) {
    if (!c.agent_last_seen) return "未见";
    return c.online ? "在线" : "离线";
  }

  function statusKind(c: Client) {
    if (!c.agent_last_seen) return "";
    return c.online ? "on" : "off";
  }

  function seenText(iso?: string | null) {
    if (!iso) return "—";
    return new Date(iso).toLocaleString();
  }

  function restoreText(c: Client) {
    if (!c.agent_last_seen) return "—";
    const mode = c.mode_name ? `（${c.mode_name}）` : "";
    if (c.frozen) return `已冻结${mode}`;
    if (c.armed) return `待重启生效${mode}`;
    return `未冻结${mode}`;
  }

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
    setExcludeTemp(true);
    setFormError("");
    setCaptureOpen(true);
  }

  function openDeploy() {
    if (selected.size === 0) {
      msg.warning("请先选择要部署的终端");
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
        assigned_ip: editAssignedIp,
      });
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
      if (detail?.id === c.id) setDetail(null);
      await refresh();
      msg.success("已删除终端");
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "删除失败");
    }
  }

  function toggleAll(checked: boolean) {
    setSelected(checked ? new Set(visible.map((c) => c.id)) : new Set());
  }

  function changeLab(id: string) {
    setFilterLabId(id);
    writeLabId(id);
  }

  function openDetail(c: Client) {
    setDetail(c);
    setEditName(c.name || "");
    setEditLabId(c.lab_id);
    setEditNotes(c.notes || "");
    setEditAssignedIp(c.assigned_ip || "");
  }

  function openStage(mode: "view" | "control") {
    if (picked.length !== 1) {
      msg.warning("请只选择一台终端");
      return;
    }
    setStage({ mac: picked[0].mac, mode });
  }

  async function refreshPresets() {
    setPresets(await api.commandPresets.list());
  }

  async function openPreset(kind: "command" | "url" | "program") {
    if (picked.length !== 1) {
      msg.warning("请只选择一台终端");
      return;
    }
    setCommandError("");
    setPresetKind(kind);
    setPresetEditing(false);
    setPresetId(null);
    try {
      await refreshPresets();
    } catch (e) {
      setCommandError(String((e as Error).message || e));
    }
  }

  function startAddPreset() {
    setPresetEditing(true);
    setPresetId(null);
    setPresetName("");
    setPresetCommand("");
    setPresetUrl("https://");
    setPresetProgram("");
    setPresetArgs("");
  }

  function startEditPreset(p: CommandPreset) {
    setPresetEditing(true);
    setPresetId(p.id);
    setPresetName(p.name);
    setPresetCommand(p.command);
    setPresetUrl(p.url || "https://");
    setPresetProgram(p.program_path);
    setPresetArgs(p.args);
  }

  async function savePreset() {
    if (!presetKind) return;
    if (!presetName.trim()) {
      setCommandError("请填写名称");
      return;
    }
    if (presetKind === "command" && !presetCommand.trim()) {
      setCommandError("请填写命令");
      return;
    }
    if (presetKind === "url" && !presetUrl.trim()) {
      setCommandError("请填写网址");
      return;
    }
    if (presetKind === "program" && !presetProgram.trim()) {
      setCommandError("请填写程序路径");
      return;
    }
    const payload = {
      kind: presetKind,
      name: presetName.trim(),
      command: presetCommand.trim(),
      url: presetUrl.trim(),
      program_path: presetProgram.trim(),
      args: presetArgs.trim(),
    };
    try {
      if (presetId) {
        await api.commandPresets.update(presetId, payload);
      } else {
        await api.commandPresets.create(payload);
      }
      msg.success("已保存");
      setPresetEditing(false);
      setPresetId(null);
      await refreshPresets();
    } catch (e) {
      setCommandError(String((e as Error).message || e));
    }
  }

  async function deletePreset(p: CommandPreset) {
    if (!window.confirm(`删除「${p.name}」？`)) return;
    try {
      await api.commandPresets.delete(p.id);
      if (presetId === p.id) {
        setPresetEditing(false);
        setPresetId(null);
      }
      await refreshPresets();
      msg.success("已删除");
    } catch (e) {
      setCommandError(String((e as Error).message || e));
    }
  }

  async function executePreset(p: CommandPreset) {
    if (p.kind === "command") {
      await sendClientCommand("run_program", {
        path: "C:\\Windows\\System32\\cmd.exe",
        args: ["/c", p.command],
      });
    } else if (p.kind === "url") {
      await sendClientCommand("open_url", { url: p.url });
    } else {
      await sendClientCommand("run_program", {
        path: p.program_path,
        args: p.args.trim() ? p.args.trim().split(/\s+/) : [],
      });
    }
  }

  async function sendClientCommand(kind: string, payload: unknown) {
    if (picked.length !== 1) {
      setCommandError("请只选择一台终端");
      return;
    }
    setCommandError("");
    try {
      await api.clients.command(picked[0].mac, kind, payload);
      msg.success("命令已通过 Agent 即时通道下发");
    } catch (e) {
      setCommandError(String((e as Error).message || e));
    }
  }

  async function openPower() {
    try {
      setPowerRows(await api.power.list());
      setPowerOpen(true);
    } catch (e) {
      msg.error(String((e as Error).message || e));
    }
  }

  function openFiles(kind: "collect" | "distribute") {
    if (picked.length === 0) {
      msg.warning("请先勾选终端");
      return;
    }
    setFormError("");
    setBatchFile(null);
    setFileKind(kind);
  }

  async function submitFiles(e: FormEvent) {
    e.preventDefault();
    const macs = picked.map((c) => c.mac);
    if (macs.length === 0) {
      setFormError("请先勾选终端");
      return;
    }
    try {
      if (fileKind === "collect") {
        const r = await api.remote.fileCollect({ macs, source_dir: collectDir });
        msg.success(`已下发收集任务（${r.targets} 台），到维护任务查看`);
      } else {
        if (!batchFile) {
          setFormError("请选择要下发的 zip 文件");
          return;
        }
        const r = await api.remote.fileDistribute(targetDir, macs, batchFile);
        msg.success(`已下发文件（${r.targets} 台），到维护任务查看`);
      }
      setFileKind(null);
    } catch (err) {
      setFormError(err instanceof Error ? err.message : "下发失败");
    }
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

  async function bulkPower(kind: "reboot" | "shutdown" | "wol") {
    const targets = clients.filter((c) => selected.has(c.id));
    if (!targets.length) {
      msg.warning("请先选择终端");
      return;
    }
    const labels = { reboot: "重启", shutdown: "关机", wol: "开机(WOL)" };
    if (
      !window.confirm(
        `对选中的 ${targets.length} 台终端下发「${labels[kind]}」？`,
      )
    ) {
      return;
    }
    try {
      const res = await api.power.createBulk(
        targets.map((c) => c.mac),
        kind,
        `ui bulk ${kind}`,
      );
      msg.success(`已下发 ${res.created} 台${labels[kind]}`);
      if (res.errors.length) msg.error(res.errors.join("; "));
    } catch (err) {
      msg.error(err instanceof Error ? err.message : "电源指令失败");
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
        exclude_temp: excludeTemp,
      });
      setCaptureOpen(false);
      msg.success(
        excludeTemp
          ? "已创建采集任务（将清除临时文件后再打包）"
          : "已创建采集任务",
      );
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
    visible.length > 0 && visible.every((c) => selected.has(c.id));
  const onlineCount = clients.filter((c) => c.online).length;
  const needOne = picked.length === 1 ? undefined : "请只选一台";
  const needSome = picked.length > 0 ? undefined : "请先勾选终端";
  const stagePeers = stage
    ? visible.some((c) => c.mac === stage.mac)
      ? visible
      : [
          ...clients.filter((c) => c.mac === stage.mac),
          ...visible.filter((c) => c.mac !== stage.mac),
        ]
    : [];

  return (
    <div className="grid" style={{ gap: "0.75rem" }}>
      <PageHeader
        title="终端"
        meta={
          query || status
            ? `${visible.length} / ${clients.length} 台 · 在线 ${onlineCount}`
            : `${clients.length} 台 · 在线 ${onlineCount}`
        }
      />
      {labs.length === 0 ? (
        <p className="muted">请先在「校区 / 机房」中创建机房，再登记终端。</p>
      ) : null}

      {stage ? (
        <AssistStage
          key={`${stage.mac}-${stage.mode}`}
          mac={stage.mac}
          mode={stage.mode}
          peers={stagePeers.map((c) => ({
            mac: c.mac,
            name: c.name,
            online: c.online,
            last_ip: c.last_ip,
          }))}
          onClose={() => setStage(null)}
          onSwitch={(mac) => setStage({ mac, mode: stage.mode })}
        />
      ) : (
        <div className="card">
          <div className="context-bar">
            <select
              aria-label="机房"
              value={filterLabId}
              onChange={(e) => changeLab(e.target.value)}
            >
              <option value="">全部机房</option>
              {labs.map((lab) => (
                <option key={lab.id} value={lab.id}>
                  {lab.name}
                </option>
              ))}
            </select>
            <input
              aria-label="搜索终端"
              value={query}
              placeholder="名称 / IP / MAC"
              onChange={(e) => setQuery(e.target.value)}
            />
            <select
              aria-label="状态"
              value={status}
              onChange={(e) => setStatus(e.target.value)}
            >
              <option value="">全部状态</option>
              <option value="online">在线</option>
              <option value="offline">离线</option>
              <option value="unfrozen">未冻结</option>
            </select>
          </div>
          <div className="command-bar">
            <div className="command-group">
              <button
                type="button"
                className="btn sm"
                onClick={openAdd}
                disabled={labs.length === 0}
              >
                添加
              </button>
              <button
                type="button"
                className="btn ghost sm"
                onClick={openBulk}
                disabled={labs.length === 0}
              >
                导入
              </button>
              <button type="button" className="btn ghost sm" onClick={openEnroll}>
                自动登记
              </button>
            </div>
            <div className="command-group">
              <button
                type="button"
                className="btn ghost sm"
                disabled={!!needSome}
                title={needSome}
                onClick={() => bulkPower("wol")}
              >
                开机
              </button>
              <button
                type="button"
                className="btn ghost sm"
                disabled={!!needSome}
                title={needSome}
                onClick={() => bulkPower("shutdown")}
              >
                关机
              </button>
              <button
                type="button"
                className="btn ghost sm"
                disabled={!!needSome}
                title={needSome}
                onClick={() => bulkPower("reboot")}
              >
                重启
              </button>
            </div>
            <div className="command-group">
              <button
                type="button"
                className="btn ghost sm"
                disabled={!!needOne}
                title={needOne}
                onClick={openCapture}
              >
                采集
              </button>
              <button
                type="button"
                className="btn ghost sm"
                disabled={!!needSome}
                title={needSome}
                onClick={openDeploy}
              >
                部署
              </button>
            </div>
            <div className="command-group">
              <button
                type="button"
                className="btn ghost sm"
                disabled={!!needOne}
                title={needOne}
                onClick={() => openStage("view")}
              >
                观察
              </button>
              <button
                type="button"
                className={`btn sm${picked.length === 1 && picked[0].online ? " accent" : ""}`}
                disabled={!!needOne}
                title={needOne}
                onClick={() => openStage("control")}
              >
                协助
              </button>
              <div className="more-menu">
                <button
                  type="button"
                  className="btn ghost sm"
                  disabled={!!needOne}
                  title={needOne}
                  aria-expanded={commandMenuOpen}
                  onClick={(e) => {
                    e.stopPropagation();
                    setCommandMenuOpen((v) => !v);
                  }}
                >
                  命令
                </button>
                {commandMenuOpen ? (
                  <div className="more-pop">
                    <button
                      type="button"
                      className="btn ghost sm"
                      onClick={() => {
                        setCommandMenuOpen(false);
                        openPreset("command");
                      }}
                    >
                      执行命令
                    </button>
                    <button
                      type="button"
                      className="btn ghost sm"
                      onClick={() => {
                        setCommandMenuOpen(false);
                        openPreset("url");
                      }}
                    >
                      打开网页
                    </button>
                    <button
                      type="button"
                      className="btn ghost sm"
                      onClick={() => {
                        setCommandMenuOpen(false);
                        openPreset("program");
                      }}
                    >
                      运行程序
                    </button>
                  </div>
                ) : null}
              </div>
            </div>
            <div className="command-group">
              <button
                type="button"
                className="btn ghost sm"
                disabled={!!needSome}
                title={needSome}
                onClick={() => openFiles("collect")}
              >
                收集
              </button>
              <button
                type="button"
                className="btn ghost sm"
                disabled={!!needSome}
                title={needSome}
                onClick={() => openFiles("distribute")}
              >
                下发
              </button>
              <button
                type="button"
                className="btn ghost sm"
                disabled={!!needSome}
                title={needSome}
                onClick={() => {
                  setMoveLabId(filterLabId || labs[0]?.id || "");
                  setFormError("");
                  setMoveOpen(true);
                }}
              >
                移动
              </button>
              <div className="more-menu">
                <button
                  type="button"
                  className="btn ghost sm"
                  aria-expanded={moreOpen}
                  onClick={(e) => {
                    e.stopPropagation();
                    setMoreOpen((v) => !v);
                  }}
                >
                  更多
                </button>
                {moreOpen ? (
                  <div className="more-pop">
                    <button
                      type="button"
                      className="btn ghost sm"
                      disabled={!!needSome}
                      title={needSome}
                      onClick={() => {
                        setMoreOpen(false);
                        navigate("/tasks", {
                          state: {
                            open: "agent",
                            macs: picked.map((c) => c.mac),
                          },
                        });
                      }}
                    >
                      更新 Agent
                    </button>
                    <button
                      type="button"
                      className="btn ghost sm"
                      onClick={() => {
                        setMoreOpen(false);
                        void openPower();
                      }}
                    >
                      电源记录
                    </button>
                  </div>
                ) : null}
              </div>
            </div>
            <span className="command-spacer" />
            <button
              type="button"
              className="btn danger sm"
              disabled={!!needSome}
              title={needSome}
              onClick={bulkDelete}
            >
              删除{picked.length ? ` (${picked.length})` : ""}
            </button>
          </div>
          <table className="data-table clickable">
            <thead>
              <tr>
                <th>
                  <input
                    type="checkbox"
                    checked={allChecked}
                    onChange={(e) => toggleAll(e.target.checked)}
                    aria-label="全选当前列表"
                  />
                </th>
                <th>状态</th>
                <th>名称</th>
                <th>IP</th>
                <th>MAC</th>
                <th>机房</th>
                <th>还原</th>
                <th>Agent</th>
                <th>最近心跳</th>
              </tr>
            </thead>
            <tbody>
              {visible.length === 0 ? (
                <tr>
                  <td colSpan={9} className="muted">
                    没有符合条件的终端。
                  </td>
                </tr>
              ) : (
                visible.map((c) => (
                  <tr
                    key={c.id}
                    className={selected.has(c.id) ? "picked" : undefined}
                    onClick={() => openDetail(c)}
                  >
                    <td onClick={(e) => e.stopPropagation()}>
                      <input
                        type="checkbox"
                        checked={selected.has(c.id)}
                        onChange={(e) => toggleOne(c.id, e.target.checked)}
                        aria-label={`选择 ${c.mac}`}
                      />
                    </td>
                    <td>
                      <span className="status">
                        <span className={`status-dot ${statusKind(c)}`} />
                        {onlineText(c)}
                      </span>
                    </td>
                    <td>{c.name || "—"}</td>
                    <td className="mono">
                      {c.last_ip || "—"}
                      {c.assigned_ip && c.assigned_ip !== c.last_ip ? (
                        <span className="ip-mark muted" title={`分配 ${c.assigned_ip}`}>
                          分配
                        </span>
                      ) : null}
                    </td>
                    <td className="mono">{c.mac}</td>
                    <td>{labMap.get(c.lab_id)?.name || c.lab_id}</td>
                    <td className="muted">{restoreText(c)}</td>
                    <td className="mono">{c.agent_version || "—"}</td>
                    <td className="muted">{seenText(c.agent_last_seen)}</td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      )}

      <Drawer
        open={detail !== null && !stage}
        title={detail?.name || detail?.mac || "终端"}
        onClose={() => setDetail(null)}
      >
        {detail ? (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              saveClient(detail.id);
            }}
          >
            <p className="muted mono">{detail.mac}</p>
            <div className="field">
              <label>名称</label>
              <input value={editName} onChange={(e) => setEditName(e.target.value)} />
            </div>
            <div className="field">
              <label>机房</label>
              <select value={editLabId} onChange={(e) => setEditLabId(e.target.value)}>
                {labs.map((lab) => (
                  <option key={lab.id} value={lab.id}>
                    {lab.name}
                  </option>
                ))}
              </select>
            </div>
            <div className="field">
              <label>分配 IP</label>
              <input
                className="mono"
                value={editAssignedIp}
                onChange={(e) => setEditAssignedIp(e.target.value)}
                placeholder="静态 IP"
              />
            </div>
            <div className="field">
              <label>上报 IP</label>
              <input className="mono" value={detail.last_ip || ""} readOnly />
            </div>
            <div className="field">
              <label>备注</label>
              <input value={editNotes} onChange={(e) => setEditNotes(e.target.value)} />
            </div>
            <p className="muted">
              {onlineText(detail)} · {restoreText(detail)} · Agent {detail.agent_version || "—"}
              <br />
              心跳 {seenText(detail.agent_last_seen)}
            </p>
            <div className="modal-actions">
              <button className="btn sm" type="submit">
                保存
              </button>
              <button
                className="btn danger sm"
                type="button"
                onClick={() => removeClient(detail)}
              >
                删除
              </button>
            </div>
          </form>
        ) : null}
      </Drawer>

      <Modal
        open={fileKind !== null}
        title={fileKind === "collect" ? "收集文件" : "下发文件"}
        onClose={() => setFileKind(null)}
      >
        <form onSubmit={submitFiles}>
          {formError ? <p className="error">{formError}</p> : null}
          <p className="muted">目标为当前勾选的 {picked.length} 台，进度在维护任务中查看。</p>
          {fileKind === "collect" ? (
            <div className="field">
              <label>收集目录</label>
              <input
                className="mono"
                value={collectDir}
                onChange={(e) => setCollectDir(e.target.value)}
                placeholder="C:\Users\Public\Documents"
                required
              />
            </div>
          ) : (
            <>
              <div className="field">
                <label>下发目标目录</label>
                <input
                  className="mono"
                  value={targetDir}
                  onChange={(e) => setTargetDir(e.target.value)}
                  placeholder="C:\Users\Public\Documents"
                  required
                />
              </div>
              <div className="field">
                <label>zip 文件</label>
                <input
                  type="file"
                  accept=".zip"
                  onChange={(e) => setBatchFile(e.target.files?.[0] ?? null)}
                />
              </div>
            </>
          )}
          <div className="modal-actions">
            <button type="button" className="btn ghost" onClick={() => setFileKind(null)}>
              取消
            </button>
            <button className="btn" type="submit">
              {fileKind === "collect" ? "收集到服务器" : "下发到客户端"}
            </button>
          </div>
        </form>
      </Modal>

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
          <label
            className="muted"
            style={{ display: "flex", gap: "0.5rem", alignItems: "flex-start" }}
          >
            <input
              type="checkbox"
              checked={excludeTemp}
              onChange={(e) => setExcludeTemp(e.target.checked)}
            />
            <span>采集前清除 pagefile / Temp 等（推荐）</span>
          </label>
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
        title="部署"
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
              确认部署
            </button>
          </div>
        </form>
      </Modal>

      <Modal
        open={powerOpen}
        title="电源记录"
        onClose={() => setPowerOpen(false)}
        wide
      >
        <table>
          <thead>
            <tr>
              <th>MAC</th>
              <th>类型</th>
              <th>状态</th>
              <th>详情</th>
            </tr>
          </thead>
          <tbody>
            {powerRows.length === 0 ? (
              <tr>
                <td className="muted" colSpan={4}>
                  暂无电源记录。
                </td>
              </tr>
            ) : (
              powerRows.map((p) => (
                <tr key={p.id}>
                  <td className="mono">{p.mac}</td>
                  <td>
                    {p.kind === "wol"
                      ? "开机"
                      : p.kind === "reboot"
                        ? "重启"
                        : "关机"}
                  </td>
                  <td>{p.state}</td>
                  <td className="muted">{p.detail}</td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </Modal>

      <Modal
        open={presetKind !== null}
        title={
          presetKind === "command"
            ? "执行命令"
            : presetKind === "url"
              ? "打开网页"
              : "运行程序"
        }
        onClose={() => {
          setPresetKind(null);
          setPresetEditing(false);
          setPresetId(null);
        }}
        wide
      >
        <p className="muted">
          目标：{picked[0]?.name || picked[0]?.mac || "—"}（{picked[0]?.mac}）。
          可从列表直接执行，也可新增/编辑常用项；执行结果不回传 stdout/退出码。
        </p>
        {commandError ? <p className="error">{commandError}</p> : null}

        {presetEditing ? (
          <div className="preset-editor">
            <div className="field">
              <label>名称</label>
              <input
                value={presetName}
                onChange={(e) => setPresetName(e.target.value)}
                placeholder="例如：查看 IP 配置"
              />
            </div>
            {presetKind === "command" ? (
              <div className="field">
                <label>命令（cmd /c）</label>
                <input
                  className="mono"
                  value={presetCommand}
                  onChange={(e) => setPresetCommand(e.target.value)}
                  placeholder="ipconfig /all"
                />
              </div>
            ) : null}
            {presetKind === "url" ? (
              <div className="field">
                <label>网址</label>
                <input
                  className="mono"
                  value={presetUrl}
                  onChange={(e) => setPresetUrl(e.target.value)}
                  placeholder="https://www.example.com"
                />
              </div>
            ) : null}
            {presetKind === "program" ? (
              <>
                <div className="field">
                  <label>程序路径</label>
                  <input
                    className="mono"
                    value={presetProgram}
                    onChange={(e) => setPresetProgram(e.target.value)}
                    placeholder="C:\Path\app.exe"
                  />
                </div>
                <div className="field">
                  <label>参数（空格分隔）</label>
                  <input
                    className="mono"
                    value={presetArgs}
                    onChange={(e) => setPresetArgs(e.target.value)}
                    placeholder="--flag value"
                  />
                </div>
              </>
            ) : null}
            <div className="modal-actions">
              <button
                className="btn ghost"
                type="button"
                onClick={() => {
                  setPresetEditing(false);
                  setPresetId(null);
                }}
              >
                取消
              </button>
              <button className="btn" type="button" onClick={() => void savePreset()}>
                保存
              </button>
            </div>
          </div>
        ) : (
          <div className="page-actions" style={{ marginBottom: "0.75rem" }}>
            <button className="btn" type="button" onClick={startAddPreset}>
              新增
            </button>
          </div>
        )}

        <table>
          <thead>
            <tr>
              <th>名称</th>
              <th>内容</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {presets.filter((p) => p.kind === presetKind).length === 0 ? (
              <tr>
                <td className="muted" colSpan={3}>
                  暂无常用项，点「新增」添加。
                </td>
              </tr>
            ) : (
              presets
                .filter((p) => p.kind === presetKind)
                .map((p) => (
                  <tr key={p.id}>
                    <td>{p.name}</td>
                    <td className="mono">
                      {p.kind === "command"
                        ? p.command
                        : p.kind === "url"
                          ? p.url
                          : `${p.program_path} ${p.args}`}
                    </td>
                    <td>
                      <div className="page-actions">
                        <button
                          className="btn"
                          type="button"
                          onClick={() => void executePreset(p)}
                        >
                          执行
                        </button>
                        <button
                          className="btn ghost"
                          type="button"
                          onClick={() => startEditPreset(p)}
                        >
                          编辑
                        </button>
                        <button
                          className="btn danger"
                          type="button"
                          onClick={() => void deletePreset(p)}
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

        <p className="muted">
          注意：命令通过当前 Agent 即时通道下发；没有交互式 shell，也不回传输出。
          请只在受控机房使用。
        </p>
      </Modal>
    </div>
  );
}
