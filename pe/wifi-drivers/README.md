# PE WiFi 驱动包

固件级 WiFi PXE **不可行**。本目录供 **有线/U 盘进入 PE 后** 加载无线网卡驱动，再由 `idv-pe-agent network-wizard` 连 Edge。

## 用法

1. 按机房金样硬件填写/替换 `manifest.json` 中的 `inf_path` 与真实签名驱动。  
2. PE 构建脚本将本目录打入 PE 根：`/wifi-drivers/`。  
3. 运行：`idv-pe-agent wifi-drivers --manifest pe/wifi-drivers/manifest.json`。

`status=placeholder` 的条目仅用于联调槽位，不能用于生产写盘。
