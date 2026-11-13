# 维护任务结束与超时收尸（已确认）

> 2026-11-12 · 用户确认方案 3；阈值默认 **12 小时**，后台 UI 可配

## 范围

1. **手动结束**：`POST /api/v1/tasks/{id}/cancel`；任务历史对 `pending`/`running` 显示「结束」。
2. **自动收尸**：Center 定时扫描；目标 `pending`/`running` 且 `updated_at` 超过阈值 → `failed`（超时无进度）。
3. **配置**：存 `ops_settings` key=`task_lifecycle`；默认 `stale_after_secs=43200`（**12h**）；`0` 关闭自动；维护任务页可改。
4. **数据修复**：立即结束库内卡住的采集任务。

## 非范围

- 新增 `cancelled` 状态（沿用 `failed` + 明确 error_message）
- Edge/PE 主动感知取消（claim 不再领取已失败任务即可）
