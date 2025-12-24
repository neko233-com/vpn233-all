# release.ps1
$versionFile = "version.txt"

if (-not (Test-Path $versionFile)) {
    "1.0.0" | Out-File -FilePath $versionFile -Encoding utf8
}

$currentVersion = (Get-Content $versionFile).Trim()
if ([string]::IsNullOrWhiteSpace($currentVersion)) {
    $currentVersion = "1.0.0"
}

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "   neko233-vpn 全自动发布工具 (PS1)" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# 1. 检查 Git 状态
Write-Host "[1/4] 检查本地代码状态..." -ForegroundColor Yellow
$gitStatus = git status --short
if ($gitStatus) {
    Write-Host $gitStatus
} else {
    Write-Host "代码库是干净的。"
}

$confirm = Read-Host "确认准备发布新版本？(y/n)"
if ($confirm -ne "y") { exit }

# 2. 升级版本号 (1.0.0 -> 1.0.1)
Write-Host ""
Write-Host "[2/4] 正在升级版本号..." -ForegroundColor Yellow
$parts = $currentVersion.Split('.')
if ($parts.Length -eq 3) {
    $patch = [int]$parts[2] + 1
    $newVersion = "$($parts[0]).$($parts[1]).$patch"
} else {
    $newVersion = "1.0.1"
}

$newVersion | Out-File -FilePath $versionFile -Encoding utf8 -NoNewline
Write-Host "版本升级: $currentVersion -> $newVersion" -ForegroundColor Green

# 3. Git 提交与打标签
Write-Host ""
Write-Host "[3/4] 提交更改并创建标签..." -ForegroundColor Yellow
$tagName = "v$newVersion"

git add .
git commit -m "chore: bump version to $newVersion"
git tag -a $tagName -m "Release $tagName"

if ($LASTEXITCODE -ne 0) {
    Write-Host "警告: 标签 $tagName 可能已存在，尝试强制覆盖..." -ForegroundColor Magenta
    git tag -f -a $tagName -m "Release $tagName"
}

# 4. 推送至 GitHub
Write-Host ""
Write-Host "[4/4] 推送至 GitHub..." -ForegroundColor Yellow
Write-Host "正在推送 main 分支..."
git push github main

Write-Host "正在推送标签 $tagName..."
git push github $tagName --force

if ($LASTEXITCODE -eq 0) {
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Green
    Write-Host "   发布成功！GitHub Action 已触发。" -ForegroundColor Green
    Write-Host "========================================" -ForegroundColor Green
    Write-Host "版本号: $newVersion"
    Write-Host "请前往 GitHub Releases 页面查看打包进度："
    Write-Host "https://github.com/neko233-Projects/neko233-vpn-all/releases" -ForegroundColor Blue
} else {
    Write-Host "[错误] 推送失败，请检查网络或权限。" -ForegroundColor Red
}

Read-Host "按回车键退出..."
