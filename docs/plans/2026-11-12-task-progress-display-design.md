# 任务进度显示（已确认）

> 2026-11-12 · 管理端 + PE + idv-agent；分母=打包/镜像体积；目标列不每 MAC 多行刷进度

## 行为

- 上报：`progress` 0–1；`detail`=`32.1/238.3 GB`；`edge_bytes`=已完成字节
- 采集：流式上传节流上报（~256MiB / 5s）；PE stderr 同文案
- 部署：idv-agent 下载镜像同样上报+本机打印
- 任务历史：进度列显示 detail；目标列一行摘要

## 分母

- 采集：打包后总字节（magic+头+payload）
- 部署：镜像 `size_bytes`
