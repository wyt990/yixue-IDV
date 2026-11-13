/** Display labels for maintenance task enums (API keeps English codes). */

const KIND: Record<string, string> = {
  capture: "采集",
  deploy: "部署",
};

const STATE: Record<string, string> = {
  pending: "等待中",
  running: "进行中",
  done: "已完成",
  failed: "失败",
  cancelled: "已取消",
};

const DISK_PLAN: Record<string, string> = {
  wipe_whole_disk: "整盘覆盖",
  wipe_target_partitions: "仅系统分区",
  write_system_only: "仅写系统区",
};

export function taskKindLabel(kind: string) {
  return KIND[kind] || kind;
}

export function taskStateLabel(state: string) {
  return STATE[state] || state;
}

export function diskPlanLabel(plan: string) {
  if (!plan) return "—";
  return DISK_PLAN[plan] || plan;
}
