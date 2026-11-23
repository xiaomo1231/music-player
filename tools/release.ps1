# 打包并发布到 GitHub Releases（本机和 GitHub Actions 共用这一个脚本）
#
# 本机用法（在仓库根目录）：
#   powershell -ExecutionPolicy Bypass -File tools/release.ps1
#   可选参数：-SkipBuild（用已有的安装包，只上传）  -DryRun（只打包和生成 latest.json，不上传）
#
# 会做的事：
#   1. 读 tauri.conf.json 的版本号，从 CHANGELOG.md 取这个版本的更新说明
#   2. 用签名私钥打 NSIS 安装包（生成 .sig 签名）
#   3. 生成 latest.json（软件“检查更新”读的就是它）
#   4. gh release create v<版本>，上传安装包、签名和 latest.json
#
# 私钥：优先用环境变量 TAURI_SIGNING_PRIVATE_KEY / TAURI_SIGNING_PRIVATE_KEY_PASSWORD，
# 没有时读本机 %USERPROFILE%\.tauri\music-player-updater.key 和 music-player-updater.password.txt。
param(
  [switch]$SkipBuild,
  [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$utf8 = New-Object System.Text.UTF8Encoding $false
$repo = 'xiaomo1231/music-player'
$assetBase = 'AuroraPlayer'   # 上传到 GitHub 的文件名用英文，中文文件名会被 GitHub 改掉

# ---- 版本号 ----
$conf = [IO.File]::ReadAllText((Join-Path $root 'src-tauri/tauri.conf.json')) | ConvertFrom-Json
$version = $conf.version
$pkg = [IO.File]::ReadAllText((Join-Path $root 'package.json')) | ConvertFrom-Json
if ($pkg.version -ne $version) { throw "package.json ($($pkg.version)) 和 tauri.conf.json ($version) 版本号不一致，先运行 tools/bump-version.ps1" }
$tag = "v$version"
Write-Host "== 发布 $tag =="

# ---- 更新说明：CHANGELOG.md 里的 “## <版本>” 一节 ----
$changelog = [IO.File]::ReadAllText((Join-Path $root 'CHANGELOG.md'))
$m = [regex]::Match($changelog, "(?ms)^##\s+v?$([regex]::Escape($version))\b[^\n]*\n(.*?)(?=^##\s|\z)")
if (-not $m.Success -or -not $m.Groups[1].Value.Trim()) { throw "CHANGELOG.md 里没有 ## $version 这一节（更新说明会显示在软件的更新弹窗里）" }
$notes = $m.Groups[1].Value.Trim() -replace "`r`n", "`n"

# ---- 发布前检查 ----
# Windows PowerShell 5.1 在 Stop 模式下会把外部命令的 stderr 当成异常，这里临时放宽
function Invoke-Quiet([scriptblock]$cmd) {
  $old = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try { $out = & $cmd 2>$null } finally { $ErrorActionPreference = $old }
  return $out
}

if (-not $DryRun) {
  Invoke-Quiet { gh release view $tag --repo $repo } | Out-Null
  if ($LASTEXITCODE -eq 0) { throw "GitHub 上已经有 $tag 了。改版本号后再发布。" }
  if (-not $env:GITHUB_ACTIONS) {
    $head = (git rev-parse HEAD).Trim()
    Invoke-Quiet { git fetch -q origin } | Out-Null
    $onRemote = Invoke-Quiet { git branch -r --contains $head }
    if (-not $onRemote) { throw "当前提交 $head 还没推送到 GitHub，先合并 / 推送再发布。" }
    if (git status --porcelain --untracked-files=no) { throw "有未提交的改动，先提交再发布。" }
  }
}

# ---- 签名私钥 ----
if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
  $keyFile = Join-Path $env:USERPROFILE '.tauri\music-player-updater.key'
  $pwFile = Join-Path $env:USERPROFILE '.tauri\music-player-updater.password.txt'
  if (-not (Test-Path $keyFile)) { throw "找不到签名私钥 $keyFile（见 docs/发布流程.md）" }
  $env:TAURI_SIGNING_PRIVATE_KEY = [IO.File]::ReadAllText($keyFile).Trim()
  if (Test-Path $pwFile) { $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [IO.File]::ReadAllText($pwFile).Trim() }
}

# ---- 打包 ----
$nsisDir = Join-Path $root 'src-tauri/target/release/bundle/nsis'
if (-not $SkipBuild) {
  if (Test-Path $nsisDir) { Remove-Item -Recurse -Force $nsisDir }
  $ErrorActionPreference = 'Continue'
  pnpm tauri build --bundles nsis
  $ErrorActionPreference = 'Stop'
  if ($LASTEXITCODE -ne 0) { throw "打包失败" }
}
$setup = Get-ChildItem $nsisDir -Filter "*_${version}_x64-setup.exe" | Select-Object -First 1
if (-not $setup) { throw "没找到 $version 的安装包（$nsisDir）" }
$sigPath = "$($setup.FullName).sig"
if (-not (Test-Path $sigPath)) { throw "没找到签名文件 $sigPath（打包时没拿到私钥？）" }

# ---- 整理上传文件 ----
$out = Join-Path $root 'release-out'
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory $out | Out-Null
$assetName = "${assetBase}_${version}_x64-setup.exe"
Copy-Item $setup.FullName (Join-Path $out $assetName)
Copy-Item $sigPath (Join-Path $out "$assetName.sig")
$signature = [IO.File]::ReadAllText($sigPath).Trim()
$url = "https://github.com/$repo/releases/download/$tag/$assetName"
$platform = [ordered]@{ signature = $signature; url = $url }
$latest = [ordered]@{
  version   = $version
  notes     = $notes
  pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
  platforms = [ordered]@{ 'windows-x86_64' = $platform; 'windows-x86_64-nsis' = $platform }
}
[IO.File]::WriteAllText((Join-Path $out 'latest.json'), ($latest | ConvertTo-Json -Depth 5), $utf8)
[IO.File]::WriteAllText((Join-Path $out 'notes.md'), $notes, $utf8)
Write-Host "安装包：$out\$assetName"

if ($DryRun) { Write-Host "DryRun：没有上传。"; exit 0 }

# ---- 上传 ----
$target = (git rev-parse HEAD).Trim()
$ErrorActionPreference = 'Continue'
gh release create $tag --repo $repo --target $target --title "极光播放器 $version" --notes-file (Join-Path $out 'notes.md') `
  (Join-Path $out $assetName) (Join-Path $out "$assetName.sig") (Join-Path $out 'latest.json')
if ($LASTEXITCODE -ne 0) { throw "上传失败" }
Write-Host "已发布：https://github.com/$repo/releases/tag/$tag"
