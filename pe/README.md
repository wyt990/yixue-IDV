# PE / PXE 启动资源（阶段 3）

本目录由 Edge 的 TFTP 与 HTTP `/pxe/` 提供。

## 文件

| 文件 | 作用 |
|------|------|
| `autoexec.ipxe` | iPXE 默认入口（与 `boot.ipxe` 同内容） |
| `boot.ipxe` | iPXE 第一跳脚本 |
| `pe.ipxe` | 维护环境入口脚本（报到 Edge） |

> iPXE 控制台字体不含中文，脚本里的 `echo` 须用 ASCII；中文说明写在注释或管理端即可。
| `undionly.kpxe` | BIOS PXE 链载 iPXE |
| `ipxe.efi` / `snponly.efi` | UEFI PXE 链载 iPXE |

## 放置 iPXE 二进制

可从 [boot.ipxe.org](https://boot.ipxe.org/) 下载：

- BIOS：`https://boot.ipxe.org/undionly.kpxe`
- UEFI：`https://boot.ipxe.org/x86_64-efi/ipxe.efi`（部分网卡可用 `snponly.efi`）

`edge.toml` 中 `boot_file`：UEFI 用 `ipxe.efi`，BIOS 用 `undionly.kpxe`。  
若开启 Secure Boot，需关闭或使用签名版（`x86_64-efi-sb`）。

## 真机验收要点

1. 机房 VLAN 仅由本 Edge 提供 DHCP（或 ProxyDHCP）
2. `edge.toml` 中 `pxe.enabled = true`，`next_server` 填 Edge 对客户机可见的 IP
3. 客户机 PXE 启动 → iPXE → `autoexec.ipxe` → `pe.ipxe` → `GET /api/v1/pe/checkin.ipxe`
4. Center 对未知 MAC 按默认机房命名规则自动登记；终端页可见
5. 无采集/部署任务时 `sanboot` 回本地盘；有任务时停留等待 PE（后续阶段）

无真实机房时，使用 `deploy/scripts/stage3-accept.ps1` 做协议级验收（DHCP Offer、TFTP、`pe-agent checkin`）。
