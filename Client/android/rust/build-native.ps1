# Build the Rust JNI bridge -> app/src/main/jniLibs/<abi>/libwtmd_android_bridge.so
# Usage: powershell -ExecutionPolicy Bypass -File Client\android\rust\build-native.ps1
# Requires: cargo-ndk 4.x, NDK 27.x under ANDROID_HOME, rustup targets
#   aarch64-linux-android / armv7-linux-androideabi / x86_64-linux-android.
# (ASCII-only on purpose: Windows PowerShell 5.1 misreads UTF-8 scripts without BOM.)
param(
    [string[]]$Targets = @("arm64-v8a", "armeabi-v7a", "x86_64")
)

$ErrorActionPreference = "Stop"
$rustDir = $PSScriptRoot
$outDir = Join-Path $rustDir "..\app\src\main\jniLibs"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$targetArgs = ($Targets | ForEach-Object { "-t $_" }) -join " "

Push-Location $rustDir
try {
    Invoke-Expression "cargo ndk $targetArgs -o `"$outDir`" build --release"
    if ($LASTEXITCODE -ne 0) { throw "cargo ndk failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}

Write-Host "OK: jniLibs written to $outDir"
Get-ChildItem -Recurse $outDir -Filter *.so | ForEach-Object { Write-Host "  $($_.FullName)" }
