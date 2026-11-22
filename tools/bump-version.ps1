# 改版本号：同时改 package.json、src-tauri/Cargo.toml、src-tauri/Cargo.lock、src-tauri/tauri.conf.json
# 用法：powershell -File tools/bump-version.ps1 0.2.0
param([Parameter(Mandatory = $true)][string]$Version)
$ErrorActionPreference = 'Stop'

if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$') {
  throw "版本号格式应为 主.次.修订，例如 0.2.0（不要带 v）"
}

$root = Split-Path -Parent $PSScriptRoot
$utf8 = New-Object System.Text.UTF8Encoding $false

function Update-File([string]$rel, [string]$pattern, [string]$replacement) {
  $path = Join-Path $root $rel
  $text = [IO.File]::ReadAllText($path)
  $re = [regex]::new($pattern, 'Multiline')
  if (-not $re.IsMatch($text)) { throw "$rel 里没找到版本号" }
  $text = $re.Replace($text, $replacement, 1)
  [IO.File]::WriteAllText($path, $text, $utf8)
  Write-Host "  $rel -> $Version"
}

Update-File 'package.json' '^(\s*"version":\s*")[^"]+(")' "`${1}$Version`${2}"
Update-File 'src-tauri/tauri.conf.json' '^(\s*"version":\s*")[^"]+(")' "`${1}$Version`${2}"
Update-File 'src-tauri/Cargo.toml' '^(version\s*=\s*")[^"]+(")' "`${1}$Version`${2}"
Update-File 'src-tauri/Cargo.lock' '(name = "music-player"\r?\nversion = ")[^"]+(")' "`${1}$Version`${2}"

Write-Host "版本号已改为 $Version。记得在 CHANGELOG.md 里加一节 ## $Version 写更新说明。"
