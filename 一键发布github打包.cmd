@echo off
setlocal enabledelayedexpansion

echo ========================================
echo   neko233-git 一键发布 GitHub Release
echo ========================================
echo.

:: 1. 检查 Git 状态
echo [1/4] 检查本地代码状态...
git status --short
echo.
set /p confirm="确认所有更改已提交并准备发布？(y/n): "
if /i "%confirm%" neq "y" exit /b

:: 2. 读取版本号
echo.
echo [2/4] 正在从 version.txt 读取版本号...
if not exist version.txt (
    set /p new_version="找不到 version.txt，请输入新版本号 (例如 1.0.0): "
    echo !new_version!> version.txt
)

set /p version=<version.txt
:: 去除可能存在的空格
set version=%version: =%

if "%version%"=="" (
    echo 错误: version.txt 内容为空
    pause
    exit /b
)

echo 检测到版本号: %version%
set tag_name=v%version%

:: 3. 创建 Tag
echo.
echo [3/4] 创建本地标签: %tag_name%
git add version.txt
git commit -m "chore: bump version to %version%" >nul 2>&1
git tag -a %tag_name% -m "Release %tag_name%"
if %errorlevel% neq 0 (
    echo.
    echo 警告: 标签 %tag_name% 可能已存在。
    set /p retry="是否尝试强制覆盖并重新发布？(y/n): "
    if /i "!retry!"=="y" (
        git tag -f -a %tag_name% -m "Release %tag_name%"
    ) else (
        exit /b
    )
)

:: 4. 推送至 GitHub
echo.
echo [4/4] 推送代码与标签至 GitHub...
echo 正在推送 main 分支...
git push origin main

echo 正在推送标签 %tag_name%...
git push origin %tag_name% --force

if %errorlevel% eq 0 (
    echo.
    echo ========================================
    echo   发布指令已发出！
    echo ========================================
    echo GitHub Action 正在后台为您打包：
    echo 1. 运行跨平台自动化测试
    echo 2. 编译 Windows/Linux/macOS 二进制
    echo 3. 自动生成 .zip 包并上传至 Release
    echo.
    echo 请稍后在 GitHub Releases 页面查看结果。
) else (
    echo.
    echo [错误] 推送失败，请检查网络连接或 GitHub 权限。
)

pause