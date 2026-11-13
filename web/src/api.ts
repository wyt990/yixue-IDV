const TOKEN_KEY = "idv_token";

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}

export function setToken(token: string | null) {
  if (token) localStorage.setItem(TOKEN_KEY, token);
  else localStorage.removeItem(TOKEN_KEY);
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (!headers.has("Content-Type") && init.body) {
    headers.set("Content-Type", "application/json");
  }
  const token = getToken();
  if (token) headers.set("Authorization", `Bearer ${token}`);

  const res = await fetch(path, { ...init, headers });
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  let data: unknown = text;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = text;
    }
  } else {
    data = null;
  }
  if (!res.ok) {
    const message =
      typeof data === "string"
        ? data
        : data && typeof data === "object" && "message" in data
          ? String((data as { message: unknown }).message)
          : text || res.statusText;
    throw new ApiError(res.status, message);
  }
  return data as T;
}

export type Campus = {
  id: string;
  name: string;
  default_image_id?: string | null;
  edge_ids?: string[];
  created_at: string;
};
export type Lab = {
  id: string;
  campus_id: string;
  name: string;
  name_prefix?: string;
  suffix_width?: number;
  suffix_start?: number;
  next_suffix?: number;
  ip_start?: string;
  ip_end?: string;
  default_image_id?: string | null;
  edge_ids?: string[];
  created_at: string;
};
export type Client = {
  id: string;
  lab_id: string;
  mac: string;
  name: string;
  notes: string;
  last_seen?: string | null;
  created_at: string;
};
export type EnrollmentSettings = {
  default_campus_id: string;
  default_lab_id: string;
};
export type TaskLifecycleSettings = {
  /** Seconds without progress before auto-fail; 0 disables. Default 43200 (12h). */
  stale_after_secs: number;
};
export type ImageItem = {
  id: string;
  name: string;
  os_family: string;
  version_label: string;
  checksum_sha256: string;
  size_bytes: number;
  storage_uri: string;
  boot_mode: string;
  notes: string;
  partition_map?: string;
  content_type?: string;
  base_image_id?: string | null;
  created_at: string;
};
export type ImageFileEntry = {
  name: string;
  size_bytes: number;
  storage_uri: string;
  checksum_sha256: string;
};
export type ModePolicy = {
  id: string;
  name: string;
  restore: string;
  system_action: string;
  persistent_action: string;
  volatile_action: string;
  notes: string;
  schedule_start?: string | null;
  schedule_end?: string | null;
  usb_restricted?: number;
  created_at: string;
  updated_at: string;
};
export type TaskTarget = {
  id: string;
  task_id: string;
  mac: string;
  state: string;
  progress: number;
  error_message: string;
  detail: string;
  p2p_bytes?: number;
  edge_bytes?: number;
  updated_at: string;
};
export type TaskView = {
  id: string;
  kind: string;
  state: string;
  image_id: string | null;
  edge_id: string | null;
  disk_plan: string;
  name: string;
  error_message: string;
  created_by: string;
  created_at: string;
  updated_at: string;
  targets: TaskTarget[];
};
export type Admin = {
  id: string;
  username: string;
  display_name: string;
  role?: string;
  created_at: string;
};
export type DashboardStats = {
  campuses: number;
  labs: number;
  clients: number;
  images: number;
  edges_online: number;
  edges_total: number;
  tasks_running: number;
};
export type DashboardEdgeRow = {
  id: string;
  name: string;
  online: boolean;
  cpu_pct: number;
  mem_used_bytes: number;
  mem_total_bytes: number;
  net_rx_bps: number;
  net_tx_bps: number;
  last_seen: string | null;
};
export type DashboardTaskRow = {
  id: string;
  name: string;
  kind: string;
  state: string;
  progress_text: string;
  error_message: string;
  updated_at: string;
};
export type DashboardLabRow = {
  id: string;
  name: string;
  campus_name: string;
  client_count: number;
  default_image_name: string;
  image_from_campus: boolean;
};
export type DashboardOverview = {
  stats: DashboardStats;
  edges: DashboardEdgeRow[];
  tasks_active: DashboardTaskRow[];
  tasks_failed: DashboardTaskRow[];
  labs: DashboardLabRow[];
};
export type EdgeNode = {
  id: string;
  name: string;
  node_token: string;
  listen_url: string;
  last_seen: string | null;
  cache_used_bytes?: number;
  cache_quota_bytes?: number;
  cache_warn_pct?: number;
  cache_level?: string;
  /** PE 采集暂存目录（Edge 本机路径）；空则用 Edge 本地默认 */
  capture_tmp_dir?: string;
  cpu_pct?: number;
  mem_used_bytes?: number;
  mem_total_bytes?: number;
  net_rx_bps?: number;
  net_tx_bps?: number;
  created_at: string;
  online: boolean;
};
export type ImageReplica = {
  id: string;
  edge_id: string;
  image_id: string;
  state: string;
  progress: number;
  local_path: string;
  checksum_sha256: string;
  error_message: string;
  updated_at: string;
};

export const api = {
  login: (username: string, password: string) =>
    request<{ token: string; username: string; display_name: string }>(
      "/api/v1/auth/login",
      { method: "POST", body: JSON.stringify({ username, password }) },
    ),
  me: () => request<Admin>("/api/v1/auth/me"),
  dashboard: () => request<DashboardOverview>("/api/v1/dashboard"),
  campuses: {
    list: () => request<Campus[]>("/api/v1/campuses"),
    create: (name: string) =>
      request<Campus>("/api/v1/campuses", {
        method: "POST",
        body: JSON.stringify({ name }),
      }),
    update: (
      id: string,
      payload:
        | string
        | {
            name: string;
            default_image_id?: string | null;
            edge_ids?: string[];
          },
    ) =>
      request<Campus>(`/api/v1/campuses/${id}`, {
        method: "PUT",
        body: JSON.stringify(
          typeof payload === "string" ? { name: payload } : payload,
        ),
      }),
    delete: (id: string) =>
      request<void>(`/api/v1/campuses/${id}`, { method: "DELETE" }),
  },
  labs: {
    list: () => request<Lab[]>("/api/v1/labs"),
    create: (
      campus_id: string,
      name: string,
      extra?: Partial<{
        name_prefix: string;
        suffix_width: number;
        suffix_start: number;
        ip_start: string;
        ip_end: string;
      }>,
    ) =>
      request<Lab>("/api/v1/labs", {
        method: "POST",
        body: JSON.stringify({ campus_id, name, ...extra }),
      }),
    update: (
      id: string,
      payload: {
        name?: string;
        campus_id?: string;
        name_prefix?: string;
        suffix_width?: number;
        suffix_start?: number;
        next_suffix?: number;
        ip_start?: string;
        ip_end?: string;
        default_image_id?: string | null;
        edge_ids?: string[];
      },
    ) =>
      request<Lab>(`/api/v1/labs/${id}`, {
        method: "PUT",
        body: JSON.stringify(payload),
      }),
    delete: (id: string) =>
      request<void>(`/api/v1/labs/${id}`, { method: "DELETE" }),
  },
  clients: {
    list: (lab_id?: string) =>
      request<Client[]>(
        lab_id
          ? `/api/v1/clients?lab_id=${encodeURIComponent(lab_id)}`
          : "/api/v1/clients",
      ),
    create: (payload: {
      lab_id: string;
      mac: string;
      name?: string;
      notes?: string;
    }) =>
      request<Client>("/api/v1/clients", {
        method: "POST",
        body: JSON.stringify(payload),
      }),
    update: (
      id: string,
      payload: { lab_id?: string; name?: string; notes?: string },
    ) =>
      request<Client>(`/api/v1/clients/${id}`, {
        method: "PUT",
        body: JSON.stringify(payload),
      }),
    delete: (id: string) =>
      request<void>(`/api/v1/clients/${id}`, { method: "DELETE" }),
    bulk: (lab_id: string, macs: string[]) =>
      request<{ created: Client[]; skipped: string[]; errors: string[] }>(
        "/api/v1/clients/bulk",
        { method: "POST", body: JSON.stringify({ lab_id, macs }) },
      ),
    bulkDelete: (ids: string[]) =>
      request<{ affected: number; errors: string[] }>(
        "/api/v1/clients/bulk-delete",
        { method: "POST", body: JSON.stringify({ ids }) },
      ),
    bulkMove: (ids: string[], lab_id: string, rename = false) =>
      request<{ affected: number; errors: string[] }>(
        "/api/v1/clients/bulk-move",
        { method: "POST", body: JSON.stringify({ ids, lab_id, rename }) },
      ),
  },
  enrollment: {
    get: () => request<EnrollmentSettings>("/api/v1/ops/enrollment"),
    set: (payload: EnrollmentSettings) =>
      request<EnrollmentSettings>("/api/v1/ops/enrollment", {
        method: "POST",
        body: JSON.stringify(payload),
      }),
  },
  taskLifecycle: {
    get: () => request<TaskLifecycleSettings>("/api/v1/ops/task-lifecycle"),
    set: (payload: TaskLifecycleSettings) =>
      request<TaskLifecycleSettings>("/api/v1/ops/task-lifecycle", {
        method: "POST",
        body: JSON.stringify(payload),
      }),
  },
  images: {
    list: () => request<ImageItem[]>("/api/v1/images"),
    create: (payload: {
      name: string;
      os_family: string;
      version_label?: string;
      checksum_sha256: string;
      size_bytes?: number;
      storage_uri?: string;
      boot_mode?: string;
      notes?: string;
    }) =>
      request<ImageItem>("/api/v1/images", {
        method: "POST",
        body: JSON.stringify(payload),
      }),
    delete: (id: string, delete_file = false) =>
      request<void>(
        `/api/v1/images/${id}?delete_file=${delete_file ? "true" : "false"}`,
        { method: "DELETE" },
      ),
    listFiles: () => request<ImageFileEntry[]>("/api/v1/images/files"),
    fileMeta: (name: string) =>
      request<ImageFileEntry>(
        `/api/v1/images/files/meta?name=${encodeURIComponent(name)}`,
      ),
    uploadFile: async (file: File) => {
      const headers = new Headers();
      const token = getToken();
      if (token) headers.set("Authorization", `Bearer ${token}`);
      headers.set("Content-Type", "application/octet-stream");
      const res = await fetch(
        `/api/v1/images/files?filename=${encodeURIComponent(file.name)}`,
        { method: "POST", headers, body: file },
      );
      const text = await res.text();
      let data: unknown = text;
      if (text) {
        try {
          data = JSON.parse(text);
        } catch {
          data = text;
        }
      }
      if (!res.ok) {
        const message =
          typeof data === "string"
            ? data
            : data && typeof data === "object" && "message" in data
              ? String((data as { message: unknown }).message)
              : text || res.statusText;
        throw new ApiError(res.status, message);
      }
      return data as ImageFileEntry;
    },
    buildDelta: (base_image_id: string, target_image_id: string, name?: string) =>
      request<ImageItem>("/api/v1/images/build-delta", {
        method: "POST",
        body: JSON.stringify({ base_image_id, target_image_id, name }),
      }),
    applyDelta: (base_image_id: string, delta_image_id: string, name?: string) =>
      request<ImageItem>("/api/v1/images/apply-delta", {
        method: "POST",
        body: JSON.stringify({ base_image_id, delta_image_id, name }),
      }),
  },
  admins: {
    list: () => request<Admin[]>("/api/v1/admins"),
  },
  edges: {
    list: () => request<EdgeNode[]>("/api/v1/edges"),
    create: (name: string, node_token?: string) =>
      request<EdgeNode>("/api/v1/edges", {
        method: "POST",
        body: JSON.stringify({ name, node_token }),
      }),
    update: (id: string, name: string, capture_tmp_dir = "") =>
      request<EdgeNode>(`/api/v1/edges/${id}`, {
        method: "PUT",
        body: JSON.stringify({ name, capture_tmp_dir }),
      }),
    delete: (id: string) =>
      request<void>(`/api/v1/edges/${id}`, { method: "DELETE" }),
    replicas: (id: string) =>
      request<ImageReplica[]>(`/api/v1/edges/${id}/replicas`),
    sync: (id: string, image_id: string, purge_first = false) =>
      request(`/api/v1/edges/${id}/sync`, {
        method: "POST",
        body: JSON.stringify({ image_id, purge_first }),
      }),
    purgeReplica: (id: string, image_id: string) =>
      request(`/api/v1/edges/${id}/replicas/${image_id}/purge`, {
        method: "POST",
      }),
    purgeAll: (id: string) =>
      request(`/api/v1/edges/${id}/purge_all`, { method: "POST" }),
    rotateToken: (id: string) =>
      request<EdgeNode>(`/api/v1/edges/${id}/rotate_token`, { method: "POST" }),
  },
  scale: {
    limits: () =>
      request<{
        max_concurrent_deploy_targets: number;
        max_pending_sync_per_edge: number;
        recommended_deploy_batch: number;
      }>("/api/v1/scale/limits"),
  },
  peSessions: {
    list: () =>
      request<
        {
          id: string;
          edge_id: string;
          mac: string;
          source: string;
          status: string;
          message: string;
          last_seen: string;
        }[]
      >("/api/v1/pe-sessions"),
  },
  tasks: {
    list: () => request<TaskView[]>("/api/v1/tasks"),
    cancel: (id: string) =>
      request<TaskView>(`/api/v1/tasks/${id}/cancel`, { method: "POST" }),
    capture: (payload: {
      mac: string;
      name?: string;
      edge_id?: string;
      os_family?: string;
      exclude_temp?: boolean;
    }) =>
      request<TaskView>("/api/v1/tasks/capture", {
        method: "POST",
        body: JSON.stringify(payload),
      }),
    previewDeploy: (payload: {
      image_id: string;
      macs: string[];
      disk_plan: string;
    }) =>
      request<{
        warning: string;
        overwrite_partitions: unknown[];
        keep_partitions: unknown[];
        image_name: string;
        disk_plan: string;
        target_count: number;
      }>("/api/v1/tasks/deploy/preview", {
        method: "POST",
        body: JSON.stringify(payload),
      }),
    deploy: (payload: {
      image_id: string;
      macs: string[];
      disk_plan: string;
      name?: string;
      edge_id?: string;
      confirm_destructive: boolean;
    }) =>
      request<TaskView>("/api/v1/tasks/deploy", {
        method: "POST",
        body: JSON.stringify(payload),
      }),
    exportCsv: async () => {
      const token = getToken();
      const res = await fetch("/api/v1/tasks/export.csv", {
        headers: token ? { Authorization: `Bearer ${token}` } : {},
      });
      if (!res.ok) throw new ApiError(res.status, await res.text());
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "idv-tasks.csv";
      a.click();
      URL.revokeObjectURL(url);
    },
  },
  modes: {
    list: () => request<ModePolicy[]>("/api/v1/modes"),
    bindings: () =>
      request<
        {
          lab_id: string;
          mode_policy_id: string;
          effective: string;
          updated_at: string;
        }[]
      >("/api/v1/modes/bindings"),
    bindLab: (labId: string, mode_policy_id: string, effective = "next_boot") =>
      request(`/api/v1/labs/${labId}/mode`, {
        method: "POST",
        body: JSON.stringify({ mode_policy_id, effective }),
      }),
    updateSchedule: (
      id: string,
      payload: {
        schedule_start?: string | null;
        schedule_end?: string | null;
        usb_restricted?: boolean;
      },
    ) =>
      request<ModePolicy>(`/api/v1/modes/${id}/schedule`, {
        method: "POST",
        body: JSON.stringify(payload),
      }),
  },
  imagesExtra: {
    buildDelta: (base_image_id: string, target_image_id: string, name?: string) =>
      request<ImageItem>("/api/v1/images/build-delta", {
        method: "POST",
        body: JSON.stringify({ base_image_id, target_image_id, name }),
      }),
    applyDelta: (base_image_id: string, delta_image_id: string, name?: string) =>
      request<ImageItem>("/api/v1/images/apply-delta", {
        method: "POST",
        body: JSON.stringify({ base_image_id, delta_image_id, name }),
      }),
    recommendDelta: (payload: {
      base_image_id: string;
      target_image_id: string;
      force_full?: boolean;
      estimated_delta_ratio?: number;
    }) =>
      request<{ recommendation: string; reason: string; content_type: string }>(
        "/api/v1/images/recommend-delta",
        { method: "POST", body: JSON.stringify(payload) },
      ),
  },
  power: {
    list: () =>
      request<
        {
          id: string;
          mac: string;
          kind: string;
          state: string;
          detail: string;
        }[]
      >("/api/v1/power/commands"),
    create: (mac: string, kind: "wol" | "shutdown", detail?: string) =>
      request("/api/v1/power/commands", {
        method: "POST",
        body: JSON.stringify({ mac, kind, detail }),
      }),
  },
  phase3: {
    getClassLock: () =>
      request<{ enabled: boolean; start_hour_utc: number; end_hour_utc: number }>(
        "/api/v1/ops/class-lock",
      ),
    setClassLock: (payload: {
      enabled: boolean;
      start_hour_utc: number;
      end_hour_utc: number;
    }) =>
      request("/api/v1/ops/class-lock", {
        method: "POST",
        body: JSON.stringify(payload),
      }),
    listSoftware: () =>
      request<{ id: string; name: string; version_label: string; size_bytes: number }[]>(
        "/api/v1/software/packages",
      ),
    createSoftware: (name: string, version_label = "") =>
      request<{ id: string }>("/api/v1/software/packages", {
        method: "POST",
        body: JSON.stringify({ name, version_label }),
      }),
    listAssist: () =>
      request<
        { id: string; mac: string; kind: string; state: string; requested_by: string }[]
      >("/api/v1/assist/sessions"),
    createAssist: (mac: string, kind: "view" | "control" = "view") =>
      request("/api/v1/assist/sessions", {
        method: "POST",
        body: JSON.stringify({ mac, kind }),
      }),
    listSchedules: () =>
      request<
        {
          id: string;
          name: string;
          kind: string;
          mac_or_tag: string;
          hour_utc: number;
          minute_utc: number;
        }[]
      >("/api/v1/ops/power-schedules"),
    auditExportUrl: "/api/v1/audit/export.csv",
  },
};
