# 阶段 6 P0 设计：Agent + 模式 + 分区级还原

## 范围（本刀）

- `idv-agent`：心跳、拉取 ModePolicy、执行分区级还原（文件盘模拟验收）
- Center：ModePolicy CRUD、绑定机房、Agent API、增量推荐引擎（规则）
- 镜像 meta 预留 `content_type` / `base_image_id`
- 二期签字页 + `stage6-accept.ps1`

## 非本刀（后续迭代）

- Windows 驱动级还原、真机卷管理
- P2P 生产做种生命周期、PE WiFi 向导、WOL 实机

## 还原模型

对本地盘（或 `--disk-file`）按镜像 `partition_map`：

- `restore=reboot|manual` 且角色 `system` → 用基线镜像覆盖该区
- `persistent` / `volatile` 按策略 keep / wipe

办公模式 `restore=off` → Agent 心跳后不改盘。
