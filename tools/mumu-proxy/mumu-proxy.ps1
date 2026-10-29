# MuMu 单窗口单IP 一键配置工具
# 双击 一键配置.bat 打开图形界面，或命令行加 -Cli 无界面运行。

[CmdletBinding()]
param(
    [switch]$Cli,
    [string]$ProxyFile = "",
    [string]$Indexes = "all",
    [switch]$ClearOnly,
    [switch]$TestOnly
)

$ErrorActionPreference = 'Continue'

# 代理列表默认查找顺序：本工具目录 -> 仓库根目录（旧的默认位置）-> 当前目录
if (-not $ProxyFile) {
    $candidates = @(
        (Join-Path $PSScriptRoot 'proxies.txt'),
        (Join-Path $PSScriptRoot '..\..\proxies.txt')
    )
    $ProxyFile = @($candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1)[0]
    if (-not $ProxyFile) { $ProxyFile = Join-Path $PSScriptRoot 'proxies.txt' }
}

$script:mumuCli = $null
$script:uiLog = $null
$script:uiForm = $null

function Find-MumuCli {
    $candidates = @(
        'C:\Program Files\Netease\MuMu\nx_main\mumu-cli.exe',
        'C:\Program Files\Netease\MuMuPlayer-12.0\shell\mumu-cli.exe',
        'C:\Program Files (x86)\Netease\MuMu\nx_main\mumu-cli.exe'
    )
    foreach ($c in $candidates) {
        if (Test-Path $c) { return $c }
    }
    $found = Get-ChildItem 'C:\Program Files\Netease', 'C:\Program Files (x86)\Netease' -Recurse -Filter mumu-cli.exe -ErrorAction SilentlyContinue |
        Select-Object -First 1 -ExpandProperty FullName
    return $found
}

function Write-Log {
    param([string]$Msg)
    $line = ('[{0}] {1}' -f (Get-Date -Format 'HH:mm:ss'), $Msg)
    if ($script:uiLog) {
        $script:uiLog.AppendText($line + "`r`n")
        $script:uiLog.SelectionStart = $script:uiLog.TextLength
        $script:uiLog.ScrollToCaret()
        [System.Windows.Forms.Application]::DoEvents()
    }
    else {
        Write-Host $line
    }
}

function Get-MumuInstances {
    $raw = (& $script:mumuCli info --vmindex all 2>&1 | Out-String).Trim()
    if ([string]::IsNullOrWhiteSpace($raw)) { return @() }
    if ($raw -match '"errcode"\s*:\s*[1-9]') { return @() }
    try { $json = $raw | ConvertFrom-Json } catch { return @() }
    $list = @()
    foreach ($p in $json.PSObject.Properties) {
        $v = $p.Value
        if ($null -eq $v -or $null -eq $v.index) { continue }
        $list += [pscustomobject]@{
            Index   = [int]$v.index
            Name    = [string]$v.name
            Android = [string]$v.android_version
            Started = [bool]($v.is_android_started -eq $true -or $v.is_process_started -eq $true)
        }
    }
    return @($list | Sort-Object Index)
}

function Get-AndroidVer {
    param([string]$Android)
    if ($Android -like '15*') { return '15' }
    if ($Android -like '12*') { return '12' }
    return 'auto'
}

function Connect-Instance {
    param([int]$Index)
    & $script:mumuCli adb --vmindex $Index --cmd "connect" 2>&1 | Out-Null
}

function Wait-InstanceReady {
    param([int]$Index)
    for ($i = 0; $i -lt 150; $i++) {
        Connect-Instance $Index
        $boot = (& $script:mumuCli adb --vmindex $Index --cmd "shell getprop sys.boot_completed" 2>&1 | Out-String).Trim()
        if ($boot -eq '1') { return $true }
        Start-Sleep -Seconds 2
        if ($script:uiLog) { [System.Windows.Forms.Application]::DoEvents() }
    }
    return $false
}

function Set-InstanceProxy {
    param([int]$Index, [string]$Proxy)
    Connect-Instance $Index
    & $script:mumuCli adb --vmindex $Index --cmd "shell settings put global http_proxy $Proxy" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 600
    $back = (& $script:mumuCli adb --vmindex $Index --cmd "shell settings get global http_proxy" 2>&1 | Out-String).Trim()
    return ($back -like "*$Proxy*")
}

function Clear-InstanceProxy {
    param([int]$Index)
    Connect-Instance $Index
    & $script:mumuCli adb --vmindex $Index --cmd "shell settings put global http_proxy :0" 2>&1 | Out-Null
}

function Launch-Instance {
    param($Inst)
    $ver = Get-AndroidVer $Inst.Android
    Write-Log ("  启动窗口 {0} (Android {1})..." -f $Inst.Index, $Inst.Android)
    & $script:mumuCli control --vmindex $Inst.Index --version $ver launch 2>&1 | Out-Null
}

function Read-ProxyList {
    param([string]$Path, [string]$Text)
    $lines = @()
    if ($Text) {
        $lines = $Text -split "`r?`n"
    }
    elseif ($Path -and (Test-Path -LiteralPath $Path)) {
        $lines = Get-Content -LiteralPath $Path
    }
    else {
        return @()
    }
    $out = @()
    foreach ($l in $lines) {
        $t = ($l -replace '#.*$', '').Trim()
        if ($t -ne '') { $out += $t }
    }
    return $out
}

function Invoke-Apply {
    param($Targets, [string[]]$Proxies)
    if (-not $Targets -or $Targets.Count -eq 0) { Write-Log '没有可配置的窗口。'; return }
    if (-not $Proxies -or $Proxies.Count -eq 0) { Write-Log '代理列表为空。'; return }
    $sorted = @($Targets | Sort-Object Index)
    if ($Proxies.Count -lt $sorted.Count) {
        Write-Log ("警告：代理 {0} 条，窗口 {1} 个，只配置前 {0} 个窗口。" -f $Proxies.Count, $sorted.Count)
    }
    $i = 0
    foreach ($t in $sorted) {
        if ($i -ge $Proxies.Count) { Write-Log ("跳过窗口 {0}（无对应代理）" -f $t.Index); continue }
        $proxy = $Proxies[$i]
        Write-Log ("窗口 {0} [{1}] -> {2}" -f $t.Index, $t.Name, $proxy)
        if (-not $t.Started) { Launch-Instance $t }
        if (Wait-InstanceReady $t.Index) {
            if (Set-InstanceProxy $t.Index $proxy) { Write-Log '  成功' } else { Write-Log '  失败：代理未生效' }
        }
        else {
            Write-Log '  启动超时，跳过'
        }
        $i++
    }
    Write-Log '完成。'
}

function Invoke-Clear {
    param($Targets)
    foreach ($t in @($Targets | Sort-Object Index)) {
        Write-Log ("清除窗口 {0} [{1}] 的代理..." -f $t.Index, $t.Name)
        if (-not $t.Started) { Launch-Instance $t }
        if (Wait-InstanceReady $t.Index) {
            Clear-InstanceProxy $t.Index
            Write-Log '  已清除'
        }
        else {
            Write-Log '  启动超时，跳过'
        }
    }
    Write-Log '完成。'
}

if ($Cli) {
    $script:mumuCli = Find-MumuCli
    if (-not $script:mumuCli) { Write-Host '未找到 mumu-cli.exe'; exit 1 }
    $all = Get-MumuInstances
    if ($Indexes -ne 'all') {
        $set = @{}
        foreach ($x in ($Indexes -split ',')) { $set[[int]$x.Trim()] = $true }
        $targets = @($all | Where-Object { $set.ContainsKey($_.Index) })
    }
    else {
        $targets = $all
    }

    if ($TestOnly) {
        $proxies = Read-ProxyList $ProxyFile ''
        foreach ($p in $proxies) {
            $ip = (& curl.exe -s --max-time 10 -x "http://$p" "https://api.ipify.org" 2>$null | Out-String).Trim()
            Write-Host ("{0} => {1}" -f $p, $ip)
        }
        exit 0
    }

    if ($ClearOnly) { Invoke-Clear $targets; exit 0 }
    $proxies = Read-ProxyList $ProxyFile ''
    Invoke-Apply $targets $proxies
    exit 0
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$script:mumuCli = Find-MumuCli
if (-not $script:mumuCli) {
    [System.Windows.Forms.MessageBox]::Show('未找到 MuMu 模拟器的 mumu-cli.exe，请确认已安装 MuMu 模拟器 12/15。', '提示')
    exit 1
}

$script:instances = @()

$form = New-Object System.Windows.Forms.Form
$form.Text = 'MuMu 单窗口单IP 一键配置'
$form.Size = New-Object System.Drawing.Size(820, 700)
$form.StartPosition = 'CenterScreen'
$form.FormBorderStyle = 'FixedSingle'
$form.MaximizeBox = $false
$script:uiForm = $form

$pad = 12

$lblProxy = New-Object System.Windows.Forms.Label
$lblProxy.Text = '代理列表（每行一个，格式 IP:端口）'
$lblProxy.Location = New-Object System.Drawing.Point($pad, 10)
$lblProxy.Size = New-Object System.Drawing.Size(360, 20)
$form.Controls.Add($lblProxy)

$script:txtProxies = New-Object System.Windows.Forms.TextBox
$script:txtProxies.Multiline = $true
$script:txtProxies.ScrollBars = 'Vertical'
$script:txtProxies.Location = New-Object System.Drawing.Point($pad, 34)
$script:txtProxies.Size = New-Object System.Drawing.Size(380, 270)
$script:txtProxies.Text = "1.2.3.4:8080`r`n5.6.7.8:8080"
$form.Controls.Add($script:txtProxies)

$script:btnLoad = New-Object System.Windows.Forms.Button
$script:btnLoad.Text = '从 proxies.txt 读取'
$script:btnLoad.Location = New-Object System.Drawing.Point($pad, 310)
$script:btnLoad.Size = New-Object System.Drawing.Size(180, 30)
$script:btnLoad.Add_Click({
    if (Test-Path -LiteralPath 'proxies.txt') {
        $script:txtProxies.Text = (Get-Content -LiteralPath 'proxies.txt' -Raw)
        Write-Log '已从 proxies.txt 读取代理列表。'
    }
    else {
        Write-Log '未找到 proxies.txt'
    }
})
$form.Controls.Add($script:btnLoad)

$script:btnSave = New-Object System.Windows.Forms.Button
$script:btnSave.Text = '保存到 proxies.txt'
$script:btnSave.Location = New-Object System.Drawing.Point($pad + 188, 310)
$script:btnSave.Size = New-Object System.Drawing.Size(180, 30)
$script:btnSave.Add_Click({
    Set-Content -LiteralPath 'proxies.txt' -Value $script:txtProxies.Text -Encoding UTF8
    Write-Log '已保存到 proxies.txt。'
})
$form.Controls.Add($script:btnSave)

$lblInst = New-Object System.Windows.Forms.Label
$lblInst.Text = '窗口列表（勾选要配置的窗口）'
$lblInst.Location = New-Object System.Drawing.Point(412, 10)
$lblInst.Size = New-Object System.Drawing.Size(380, 20)
$form.Controls.Add($lblInst)

$script:clbInstances = New-Object System.Windows.Forms.CheckedListBox
$script:clbInstances.CheckOnClick = $true
$script:clbInstances.Location = New-Object System.Drawing.Point(412, 34)
$script:clbInstances.Size = New-Object System.Drawing.Size(380, 270)
$form.Controls.Add($script:clbInstances)

$script:btnRefresh = New-Object System.Windows.Forms.Button
$script:btnRefresh.Text = '刷新'
$script:btnRefresh.Location = New-Object System.Drawing.Point(412, 310)
$script:btnRefresh.Size = New-Object System.Drawing.Size(120, 30)
$script:btnRefresh.Add_Click({ Refresh-InstanceList })
$form.Controls.Add($script:btnRefresh)

$script:btnAll = New-Object System.Windows.Forms.Button
$script:btnAll.Text = '全选'
$script:btnAll.Location = New-Object System.Drawing.Point(540, 310)
$script:btnAll.Size = New-Object System.Drawing.Size(120, 30)
$script:btnAll.Add_Click({
    for ($k = 0; $k -lt $script:clbInstances.Items.Count; $k++) { $script:clbInstances.SetItemChecked($k, $true) }
})
$form.Controls.Add($script:btnAll)

$script:btnNone = New-Object System.Windows.Forms.Button
$script:btnNone.Text = '全不选'
$script:btnNone.Location = New-Object System.Drawing.Point(668, 310)
$script:btnNone.Size = New-Object System.Drawing.Size(120, 30)
$script:btnNone.Add_Click({
    for ($k = 0; $k -lt $script:clbInstances.Items.Count; $k++) { $script:clbInstances.SetItemChecked($k, $false) }
})
$form.Controls.Add($script:btnNone)

$script:btnApply = New-Object System.Windows.Forms.Button
$script:btnApply.Text = '一键配置'
$script:btnApply.Location = New-Object System.Drawing.Point($pad, 356)
$script:btnApply.Size = New-Object System.Drawing.Size(180, 40)
$script:btnApply.Add_Click({
    $proxies = Read-ProxyList '' $script:txtProxies.Text
    $sel = @()
    foreach ($idx in $script:clbInstances.CheckedIndices) { $sel += $script:instances[$idx] }
    Invoke-Apply $sel $proxies
})
$form.Controls.Add($script:btnApply)

$script:btnClear = New-Object System.Windows.Forms.Button
$script:btnClear.Text = '一键清除'
$script:btnClear.Location = New-Object System.Drawing.Point($pad + 188, 356)
$script:btnClear.Size = New-Object System.Drawing.Size(180, 40)
$script:btnClear.Add_Click({
    $sel = @()
    foreach ($idx in $script:clbInstances.CheckedIndices) { $sel += $script:instances[$idx] }
    Invoke-Clear $sel
})
$form.Controls.Add($script:btnClear)

$script:btnTest = New-Object System.Windows.Forms.Button
$script:btnTest.Text = '测试代理出口IP'
$script:btnTest.Location = New-Object System.Drawing.Point($pad + 376, 356)
$script:btnTest.Size = New-Object System.Drawing.Size(180, 40)
$script:btnTest.Add_Click({
    $proxies = Read-ProxyList '' $script:txtProxies.Text
    foreach ($p in $proxies) {
        Write-Log ("测试 $p ...")
        $ip = (& curl.exe -s --max-time 10 -x "http://$p" "https://api.ipify.org" 2>$null | Out-String).Trim()
        if ($ip) { Write-Log ("  $p => $ip") } else { Write-Log ("  $p => 无法连通") }
    }
})
$form.Controls.Add($script:btnTest)

$lblLog = New-Object System.Windows.Forms.Label
$lblLog.Text = '运行日志'
$lblLog.Location = New-Object System.Drawing.Point($pad, 404)
$lblLog.Size = New-Object System.Drawing.Size(200, 20)
$form.Controls.Add($lblLog)

$script:txtLog = New-Object System.Windows.Forms.TextBox
$script:txtLog.Multiline = $true
$script:txtLog.ReadOnly = $true
$script:txtLog.ScrollBars = 'Vertical'
$script:txtLog.Location = New-Object System.Drawing.Point($pad, 428)
$script:txtLog.Size = New-Object System.Drawing.Size(776, 220)
$form.Controls.Add($script:txtLog)
$script:uiLog = $script:txtLog

function Refresh-InstanceList {
    $script:instances = Get-MumuInstances
    $script:clbInstances.Items.Clear()
    foreach ($it in $script:instances) {
        $state = if ($it.Started) { '运行中' } else { '未运行' }
        $label = ('{0,3} | {1} | Android {2} | {3}' -f $it.Index, $it.Name, $it.Android, $state)
        $script:clbInstances.Items.Add($label) | Out-Null
        $script:clbInstances.SetItemChecked($script:clbInstances.Items.Count - 1, $true)
    }
    Write-Log ("已检测到 {0} 个窗口。" -f $script:instances.Count)
}

Refresh-InstanceList
Write-Log ('MuMu 路径: {0}' -f $script:mumuCli)
Write-Log '把代理填到左侧，勾选窗口后点“一键配置”即可。'

$form.ShowDialog() | Out-Null
