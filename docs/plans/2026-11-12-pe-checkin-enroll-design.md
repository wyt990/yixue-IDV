# PE 报到即登记终端

> 2026-11-12 · 已确认（方案 1）

## 目标

iPXE / pe-agent 向 Edge 报到时，Center 对未知 MAC 按默认机房命名规则自动入库；已存在则刷新 `last_seen`。真机整盘采集 PE 后置。

## 行为

1. Edge `checkin` → `report_pe_session` → Center `POST /api/v1/edge/pe-sessions`
2. Center upsert PE 会话后调用与 Agent 心跳共用的 `ensure_client_enrolled`
3. 响应带回 `client_id` / `client_name` / `lab_id` / `created`
4. Edge checkin JSON 增加上述字段；另提供 `GET /api/v1/pe/checkin.ipxe` 供 iPXE `chain` 显示登记名并 sanboot
5. 未配置默认机房：报到成功、不建终端（日志警告）

## 非目标

- 本轮不做真机整盘采集 PE
- 不在 DHCP 阶段单独登记（避免与报到重复）
