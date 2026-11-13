# 阶段 6d 设计：真机卷钩子骨架 · 信创矩阵 · WiFi 驱动包

## 范围

| 项 | 做法 | 验收 |
|----|------|------|
| 真机卷 / 开机钩子 | `volume` 后端：`file`（原有）+ `windows-lab`（目录卷模拟）；`install-boot-hook` 写钩子脚本与登记清单 | stage6d-accept |
| 信创浏览器矩阵 | 模块 13 + 外观页提示；CSS 避免依赖过新特性 | 文档存在且列浏览器 |
| PE WiFi 驱动包 | `pe/wifi-drivers/manifest.json` + pe-agent 校验列表 | list 输出型号槽位 |

## 非本刀（仍需真机）

- 原始磁盘扇区写 / Windows VSS / 过滤驱动  
- 真实无线网卡 `.inf` 签名与 PE 注入实测  
- 全量信创浏览器自动化截图矩阵
