# 启用有线网卡的 Wake-on-LAN（客户端管理员执行）
#
# 作用：
#   1) 对每块有线网卡启用 "Wake on Magic Packet"、关闭 "Wake on Pattern"；
#   2) 关闭 Windows 快速启动（快速启动会让"关机"变成混合关机，部分网卡因此不监听 WOL）；
#   3) 打印验证结果与 BIOS 检查项。
#
#   cd "C:\Program Files\YixueIDV"
#   .\scripts\enable-wol.ps1
$ErrorActionPreference = "Stop"

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($id)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw "必须以管理员身份运行。"
}

$wired = Get-NetAdapter -Physical -ErrorAction SilentlyContinue |
  Where-Object { $_.MediaType -ne "802.11" -and $_.InterfaceDescription -notmatch "Wi-?Fi|Wireless" }

if (-not $wired) {
  Write-Host "未找到有线网卡。WOL 只支持有线以太网，请检查网卡/驱动。"
} else {
  foreach ($a in $wired) {
    Write-Host "处理网卡: $($a.Name)  MAC=$($a.MacAddress)  Status=$($a.Status)"
    Write-Host "  驱动: $($a.DriverProvider) $($a.DriverVersion) ($($a.DriverDate))"
    try {
      Set-NetAdapterPowerManagement -Name $a.Name `
        -WakeOnMagicPacket Enabled -WakeOnPattern Disabled -ErrorAction Stop | Out-Null
      Write-Host "  已尝试设置 WakeOnMagicPacket=Enabled, WakeOnPattern=Disabled"
    } catch {
      Write-Host "  WARN: 该驱动不支持通过命令设置，请在设备管理器 → 网卡 → 属性 → 电源管理 中勾选："
      Write-Host "        「允许此设备唤醒计算机」+「只允许幻数据包唤醒计算机」"
    }
    $pm = Get-NetAdapterPowerManagement -Name $a.Name -ErrorAction SilentlyContinue
    if ($pm -and "$($pm.WakeOnMagicPacket)" -eq "Unsupported") {
      Write-Host "  !! 关键问题: Windows 报告该网卡 WakeOnMagicPacket=Unsupported。"
      Write-Host "     说明驱动/BIOS 没有向系统暴露 WOL 能力，服务端发什么都唤不醒。"
      Write-Host "     处理顺序: 1) BIOS 开启 Wake on LAN/Power On by PCIe 并关闭 ErP；"
      Write-Host "               2) 安装主板/Realtek 官方网卡驱动（Windows 自带驱动常无 WOL 项）；"
      Write-Host "               3) 设备管理器→网卡→电源管理 勾选两个唤醒选项。"
    }
  }
}

# 关闭休眠/快速启动；失败不应中断（部分精简系统没有休眠）。
try {
  & powercfg /h off
  if ($LASTEXITCODE -eq 0) { Write-Host "已关闭快速启动/休眠（powercfg /h off）" }
  else { Write-Host "WARNING: powercfg /h off 返回 $LASTEXITCODE（可能已关闭或系统不支持）" }
} catch {
  Write-Host "WARNING: powercfg /h off 失败，请手动在「电源选项 → 选择电源按钮的功能」关闭快速启动"
}

Write-Host ""
Write-Host "当前网卡电源管理能力："
Get-NetAdapterPowerManagement -ErrorAction SilentlyContinue |
  Select-Object Name, WakeOnMagicPacket, WakeOnPattern, DeviceSleepOnDisconnect |
  Format-Table -AutoSize

Write-Host "系统允许唤醒的设备（powercfg /devicequery wake_armed）："
try {
  & powercfg /devicequery wake_armed | Out-Host
} catch {
  Write-Host "WARNING: powercfg 查询失败"
}

Write-Host "与 WOL 相关的网卡高级属性："
foreach ($a in $wired) {
  $props = Get-NetAdapterAdvancedProperty -Name $a.Name -ErrorAction SilentlyContinue |
    Where-Object { $_.DisplayName -match "Wake|WOL|Shutdown|PME|Magic|唤醒|关机|魔术|网络唤醒" }
  if ($props) {
    $props | Select-Object Name, DisplayName, DisplayValue | Format-Table -AutoSize
  } else {
    Write-Host "  (无) 驱动未暴露任何 WOL 相关高级属性，通常说明当前网卡驱动不支持 WOL，"
    Write-Host "       请安装 Realtek/主板厂商官方驱动后再试。"
  }
}

Write-Host "还需要确认："
Write-Host "  - BIOS：启用 Wake on LAN / Power On by PCIe，关闭 ErP/Deep Sleep（不同主板名称不同）"
Write-Host "  - 必须是有线连接；关机后网口指示灯应保持常亮"
Write-Host "  - 交换机/VLAN 不隔离广播；WOL 包由 Center 或 Edge 向本机所在子网的广播地址发送"
Write-Host "ENABLE_WOL_OK"
