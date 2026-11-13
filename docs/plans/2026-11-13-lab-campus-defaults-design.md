# 机房/校区默认镜像与边缘绑定（方案 B）

> 2026-11-13 · 已确认实现

## 目标

- 机房可配置默认基线镜像、绑定 1～N 个 Edge（priority）。
- 校区可配置默认镜像、绑定 Edge 池；机房未配置时回落校区。
- Agent 基线、任务未显式指定 Edge 时使用解析结果。

## 解析

- 镜像：`lab.default_image_id` → `campus.default_image_id` → `None`
- Edge 列表：机房 `lab_edges`（非空）→ 否则校区 `campus_edges` → 空（任意）
- 任务显式 `image_id` / `edge_id` 优先

## 数据

- `campuses.default_image_id` / `labs.default_image_id`（ON DELETE SET NULL）
- `campus_edges` / `lab_edges`（campus/lab + edge + priority）

## UI

- 校区/机房设置：默认镜像下拉 + 边缘多选（按勾选顺序写 priority）
