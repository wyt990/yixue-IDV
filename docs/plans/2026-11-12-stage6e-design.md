# 阶段 6e 设计：软件关门（审计 · 考试回落 · WOL）

## 范围

| 项 | 做法 |
|----|------|
| 模式变更审计 | `audit_events` + `GET /api/v1/audit`；bind / schedule / exam_fallback 写入 |
| 考试窗外回落教学 | heartbeat 发现 exam 且窗外 → 绑定切 teaching + 审计 |
| WOL | `idv_common::wol` 组包并广播；power API 复用 |

## 阶段 6 软件关门定义

自动化 `stage6`～`stage6e` 全绿；真机扇区/签名驱动/信创实检列入 **阶段 6 硬件试点**（模块 14），不挡软件关门。
