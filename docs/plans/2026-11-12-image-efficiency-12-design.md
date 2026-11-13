# 镜像效率 ①+② + 同机入库（已确认）

> 2026-11-12 · 用户确认：NTFS 优先跳空闲；边传边校验；同机 rename/hardlink/move

## 范围

1. **同机快速入库**：Edge `capture-tmp` 与 Center `images_dir` 同机时，`rename` → `hardlink` → `copy`，避免 HTTP 再搬一遍。
2. **② 边传边校验**：PE 只读盘一遍；头里 `payload_sha256` 先占位 64 个 `0`；Center/Edge 收妥后算哈希并原地补丁头字段。
3. **① 跳空闲（NTFS 优先）**：解析 GPT；小分区（ESP/MSR 等）整段收录；NTFS 读 `$Bitmap` 只传已用 cluster；头增加 `encoding` / `logical_size`。

## 非范围

- CDC（③）本轮不做。

## 验收

- 同机采集入库不再出现数小时 localhost HTTP 拷贝。
- PE 日志无 pass1 整盘 hashing，仅一轮上传读盘。
- NTFS 金盘镜像体积明显小于整盘；部署可还原为原逻辑盘大小。
