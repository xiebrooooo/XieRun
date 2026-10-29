<#
.SYNOPSIS
    注册/更新一个 Windows 计划任务：每天固定时刻调用 定时跑步.cmd 跑一次。

.EXAMPLE
    # 天津包，每天早上 7:30 触发，跑 2.0km / 6'30"/km，出发时刻定在当天 07:32
    .\注册每日任务.ps1 -At 07:30 -Exe "D:\运动世界\NekoSportsWorldTool-个人免卡版-20261008\dist\个人免卡版-天津-20261015-v11\运动规划.exe" -Dist 2.0 -Pace 390 -When 07:32

.EXAMPLE
    # 湖南包，21:00 触发，随机「1 小时前出发」，并允许到点唤醒电脑
    .\注册每日任务.ps1 -At 21:00 -Exe "D:\...\运动规划-TLS修复版-20261015\运动规划.exe" -Dist 2.5 -Pace 420 -When 60 -Wake

.EXAMPLE
    # 先干跑验证（不会真的提交记录）
    .\注册每日任务.ps1 -At 07:30 -Exe "..." -DryRun -Name "测试-运动规划"
    Start-ScheduledTask -TaskName "测试-运动规划"      # 跑完看 .\logs\*.log

.EXAMPLE
    # 取消
    .\注册每日任务.ps1 -Remove -Name "运动规划-天津"
#>
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$At,          # 计划任务触发时刻 HH:MM
    [string]$Dist = "2.0",                              # 距离（km）
    [string]$Pace = "390",                              # 配速（秒/km，390 = 6'30"）
    [string]$When = "",                                 # 出发时刻 HH:MM，或「多少分钟前」的分钟数；空=60
    [string]$Name = "",                                 # 任务名，空则按学校目录自动取名
    [switch]$Wake,                                      # 到点唤醒电脑（需电源计划允许唤醒定时器）
    [switch]$DryRun,                                     # 只打印不真跑（写日志但不提交记录）
    [switch]$Remove                                       # 删除同名任务
)

$ErrorActionPreference = 'Stop'
$runner = Join-Path $PSScriptRoot '定时跑步.cmd'
if (-not (Test-Path $runner)) { throw "找不到 $runner（请和 注册每日任务.ps1 放在同一目录）" }

if ([string]::IsNullOrWhiteSpace($Name)) {
    $leaf = Split-Path (Split-Path $Exe -Parent) -Leaf
    $Name = "运动规划-每日跑步-$leaf"
}

if ($Remove) {
    Unregister-ScheduledTask -TaskName $Name -Confirm:$false
    Write-Host "已删除计划任务：$Name"
    return
}

if (-not (Test-Path $Exe)) { throw "找不到 exe：$Exe" }
# 计划任务触发时刻形如 07:30 才合法
if ($At -notmatch '^([01]?\d|2[0-3]):[0-5]\d$') { throw "-At 需要 HH:MM（例如 07:30），收到：$At" }
if ($Dist -notmatch '^\d+(\.\d+)?$') { throw "-Dist 需要数字（例如 2.0）" }
if ($Pace -notmatch '^\d+$') { throw "-Pace 需要秒/km 整数（例如 390）" }

$dry = ''
if ($DryRun) { $dry = ' --test' }
# cmd /c ""脚本" "exe" 距离 配速 时刻[ --test]"
$argline = '/c ""{0}" "{1}" {2} {3} {4}{5}"' -f $runner, $Exe, $Dist, $Pace, $When, $dry

$action = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument $argline
$trigger = New-ScheduledTaskTrigger -Daily -At $At
$settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes 30) `
    -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
if ($Wake) { $settings.WakeToRun = $true }

Register-ScheduledTask -TaskName $Name -Action $action -Trigger $trigger -Settings $settings `
    -Description "运动规划：每天 $At 跑一次（$Exe）" | Out-Null

$info = Get-ScheduledTask -TaskName $Name | Get-ScheduledTaskInfo
Write-Host "已注册：$Name"
Write-Host "  触发：每天 $At   （StartWhenAvailable：错过会在开机后补跑）"
Write-Host "  执行：$Exe  --dist $Dist --pace $Pace $When$dry"
Write-Host "  日志：$(Join-Path $PSScriptRoot 'logs')"
Write-Host "  状态：$($info.LastTaskResult)  下次：$($info.NextRunTime)"
Write-Host ""
Write-Host "立刻试跑（推荐先加 -DryRun 注册一个测试任务）：Start-ScheduledTask -TaskName `"$Name`""
Write-Host "查看日志：Get-Content `"$(Join-Path $PSScriptRoot 'logs')\$(Get-Date -Format yyyyMMdd).log`" -Tail 30"
