param(
    [string]$Version = "0.4.5"
)

$ErrorActionPreference = "Stop"
$workspace = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$profileRoot = [Environment]::GetFolderPath("UserProfile")
$portableTarget = Join-Path $workspace "target\portable"
$distribution = Join-Path $workspace "dist"
$artifact = Join-Path $distribution "MINMAXXER-v$Version-windows-x64.exe"

$previousTarget = $env:CARGO_TARGET_DIR
$previousRustFlags = $env:CARGO_ENCODED_RUSTFLAGS
$previousCFlags = $env:CFLAGS
$previousCxxFlags = $env:CXXFLAGS

try {
    $separator = [char]0x1f
    $env:CARGO_TARGET_DIR = $portableTarget
    $env:CARGO_ENCODED_RUSTFLAGS = @(
        "-C",
        "target-feature=+crt-static",
        "--remap-path-prefix=$workspace=.",
        "--remap-path-prefix=$profileRoot=."
    ) -join $separator
    # Keep native dependency source paths reproducible without remapping build outputs.
    # MSVC applies /pathmap to /Fd PDB paths too, so mapping the workspace breaks the
    # CMake compiler probe used by openvr_sys when its output lives under target/.
    $env:CFLAGS = "/experimental:deterministic /MT /pathmap:`"$profileRoot`"=."
    $env:CXXFLAGS = "/experimental:deterministic /MT /pathmap:`"$profileRoot`"=."

    & cargo clean --target-dir $portableTarget
    if ($LASTEXITCODE -ne 0) {
        throw "Could not clean the isolated portable target (exit code $LASTEXITCODE)"
    }

    & cargo build --release --locked -p minmaxxer
    if ($LASTEXITCODE -ne 0) {
        throw "Portable release build failed with exit code $LASTEXITCODE"
    }

    New-Item -ItemType Directory -Path $distribution -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $portableTarget "release\minmaxxer.exe") -Destination $artifact -Force
    Get-FileHash -Algorithm SHA256 -LiteralPath $artifact
}
finally {
    $env:CARGO_TARGET_DIR = $previousTarget
    $env:CARGO_ENCODED_RUSTFLAGS = $previousRustFlags
    $env:CFLAGS = $previousCFlags
    $env:CXXFLAGS = $previousCxxFlags
}
