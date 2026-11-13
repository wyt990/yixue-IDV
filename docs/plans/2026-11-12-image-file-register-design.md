# 镜像登记：服务端文件选择 + 上传

> 2026-11-12 · 已确认

## 流程
1. 列表/上传 `images_dir` 文件
2. 选中后服务端计算 SHA-256
3. 填写显示名后登记元数据

## API
- GET `/api/v1/images/files`
- GET `/api/v1/images/files/meta?name=`
- POST `/api/v1/images/files?filename=`（octet-stream）
