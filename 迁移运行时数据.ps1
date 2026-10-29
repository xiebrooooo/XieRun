<#
.SYNOPSIS
    把「运行时数据」从旧版本目录迁到新版本目录（换包升级时用一次）。

.DESCRIPTION
    只搬运行时数据，绝不搬 exe/使用说明：
      accounts.json / account_bindings.json / config.json / identity.json
      session.json / session_registry.json / batch_state.json
      points_cache.json / fence_cache.json / policy_cache.json
      history-*.json / history-*.osm / networks\ 目录
    - config.json 里的 osm_path 若指向旧目录下的文件，会自动改写成新目录。
    - 目标目录必须已经有 运动规划.exe，否则拒绝执行（防止搬错地方）。
    - 绝不写入 zip；这些文件含凭据，只在本机目录间搬。

.EXAMPLE
    .\迁移运行时数据.ps1 -From "D:\...\dist\辽源职业技术学院-20261015-v11" -To "D:\...\dist\辽源职业技术学院-20261016-v12"
    .\迁移运行时数据.ps1 -From "<旧>" -To "<新>" -DryRun      # 只列清单
    .\迁移运行时数据.ps1 -From "<旧>" -To "<新>" -Move        # 搬走而不是复制
#>
param(
    [Parameter(Mandatory = $true)][string]$From,
    [Parameter(Mandatory = $true)][string]$To,
    [switch]$Move,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$From = (Resolve-Path $From).Path
if (-not (Test-Path $To)) { New-Item -ItemType Directory -Path $To | Out-Null }
$To = (Resolve-Path $To).Path

if ($From -eq $To) { throw "源目录和目标目录相同：$To" }
$exe = Get-ChildItem $To -Filter *.exe -ErrorAction SilentlyContinue
if (-not $exe) { throw "$To 里没有 exe，先把这个版本解出来再迁移" }

$fixed = @(
    'accounts.json', 'account_bindings.json', 'config.json', 'identity.json',
    'session.json', 'session_registry.json', 'batch_state.json',
    'points_cache.json', 'fence_cache.json', 'policy_cache.json'
)
$patterns = @('history-*.json', 'history-*.osm')

$items = @()
foreach ($n in $fixed) {
    $p = Join-Path $From $n
    if (Test-Path $p) { $items += Get-Item $p }
}
foreach ($pat in $patterns) {
    $items += Get-ChildItem $From -Filter $pat -File -ErrorAction SilentlyContinue
}
$netSrc = Join-Path $From 'networks'
$hasNet = Test-Path $netSrc

if ($items.Count -eq 0 -and -not $hasNet) {
    Write-Host "源目录里没有可迁移的运行时数据：$From"
    return
}

foreach ($f in $items) {
    $dst = Join-Path $To $f.Name
    $verb = if ($DryRun) { 'would ' + $(if ($Move) { 'move' } else { 'copy' }) } else { if ($Move) { 'move' } else { 'copy' } }
    Write-Host ("{0,-9} {1}" -f $verb, $f.Name)
    if (-not $DryRun) {
        if ($Move) { Move-Item -LiteralPath $f.FullName -Destination $dst -Force }
        else { Copy-Item -LiteralPath $f.FullName -Destination $dst -Force }
    }
}
if ($hasNet) {
    Write-Host ("{0,-9} {1}" -f $(if ($DryRun) { 'would sync' } else { 'sync' }), 'networks\')
    if (-not $DryRun) {
        New-Item -ItemType Directory -Path (Join-Path $To 'networks') -Force | Out-Null
        Copy-Item (Join-Path $netSrc '*') -Destination (Join-Path $To 'networks') -Force -Recurse
        if ($Move) { Remove-Item $netSrc -Recurse -Force }
    }
}

# config.json 里的 osm_path / history_path 若指向旧目录，改写成新目录，否则换包后路网会失效
$cfgPath = Join-Path $To 'config.json'
if ((-not $DryRun) -and (Test-Path $cfgPath)) {
    $cfg = Get-Content $cfgPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $changed = @()
    foreach ($key in @('osm_path', 'history_path')) {
        $val = $cfg.$key
        if ($val -and $val.StartsWith($From, [System.StringComparison]::OrdinalIgnoreCase)) {
            $new = $To + $val.Substring($From.Length)
            $cfg.$key = $new
            $changed += "$key -> $new"
        }
    }
    if ($changed.Count) {
        $cfg | ConvertTo-Json -Depth 8 | Set-Content $cfgPath -Encoding UTF8
        Write-Host "已改写 config.json：$($changed -join '；')"
    }
}

Write-Host ""
Write-Host "完成。目标目录：$To"
Write-Host "提示：这些文件含账号凭据，不要打进发给别人的 zip。"
