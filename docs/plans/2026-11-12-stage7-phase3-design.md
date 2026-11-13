# 阶段 7（三期）设计：运维增值能力软件骨架

> 用户要求与二期一并联调测试。真机桌面流/签名驱动仍非本刀。

## 范围（本刀可验收）

| 能力 | 实现 | 验收 |
|------|------|------|
| 多管理员角色 | `admins.role`: super / lab_admin / readonly；危险写操作拦 readonly | stage7-accept |
| 终端标签 | `client_tags`；按标签筛选 | API |
| 软件分发 | `software_packages` + `software_jobs`；agent 领取/上报 | agent + accept |
| 远程协助信令 | `assist_sessions`（view/control）；无真 VNC，会话状态机 | accept |
| 上课时段只读 | `ops_settings.class_lock`；锁定期拒创建部署**（可 force） | accept |
| 定时电源策略 | `power_schedules`；center 扫描入队 WOL/shutdown | accept |
| 审计导出 | `GET /api/v1/audit/export.csv` | accept |

## 非本刀

- 真同屏画面 / 远程键鼠驱动  
- MSI/静默安装真执行（agent 仅下载+记录模拟 apply）  
- 跨 VLAN WOL、802.1X  
