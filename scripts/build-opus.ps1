# 预编译 libopus，供 audiopus_sys 静态链接（Windows + VS CMake）
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$cmake = "C:\Program Files\Microsoft Visual Studio\18\Insiders\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
if (-not (Test-Path $cmake)) {
    $cmake = (Get-Command cmake -ErrorAction SilentlyContinue).Source
}
if (-not $cmake) {
    throw "未找到 cmake，请安装 Visual Studio 的 CMake 组件或将其加入 PATH"
}

$opusSrc = Join-Path $env:USERPROFILE ".cargo\registry\src\index.crates.io-1949cf8c6b5b557f\audiopus_sys-0.2.2\opus"
if (-not (Test-Path $opusSrc)) {
    throw "未找到 audiopus_sys 内置 opus 源码，请先 cargo fetch"
}

$buildDir = Join-Path $Root "target\opus-build"
$installDir = Join-Path $buildDir "install"
if (Test-Path $buildDir) {
    Remove-Item -Recurse -Force $buildDir
}
New-Item -ItemType Directory -Force -Path $buildDir | Out-Null

& $cmake -S $opusSrc -B $buildDir "-DCMAKE_POLICY_VERSION_MINIMUM=3.5" -DCMAKE_BUILD_TYPE=Release "-DCMAKE_INSTALL_PREFIX=$installDir"
& $cmake --build $buildDir --config Release
& $cmake --install $buildDir --config Release

Write-Host "libopus 已安装到: $installDir"
