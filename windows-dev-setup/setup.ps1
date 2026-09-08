#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Bootstraps a Windows dev environment for building and running Mnemos from
    source: Rust, VS Build Tools (C++ workload), LLVM/clang (ARM64 hosts
    only), Node/pnpm, Python/uv, and the Claude Code CLI.

.DESCRIPTION
    This is not an installer for the app — it sets up the toolchain needed to
    *build* it from source. A packaged end-user installer (no compiler, no
    Python, just double-click and run) is separate, unfinished work — see
    product_docs/WINDOWS_PARITY_AUDIT.md.

    Every step checks whether the tool is already present before installing,
    so it's safe to re-run after a partial/failed run.

    Run from an elevated PowerShell (Run as Administrator):
        .\setup.ps1

.NOTES
    Written and verified against a real Windows 11 ARM64 VM (Parallels on
    Apple Silicon) — see product_docs/WINDOWS_PARITY_AUDIT.md for what was
    actually exercised vs. what's still unverified on x64 hardware.
#>

$ErrorActionPreference = "Stop"

function Write-Step($msg) {
    Write-Host ""
    Write-Host "==> $msg" -ForegroundColor Cyan
}

function Refresh-Path {
    # Installers add to the Machine PATH, but *this* process's environment
    # was captured at shell startup and won't see it — re-read it here so
    # later steps in the same script session can find newly-installed tools
    # without requiring a new window (a real contributor running this
    # interactively will still need a fresh shell afterward for their own
    # future sessions; this only helps the rest of this script run).
    $machine = [Environment]::GetEnvironmentVariable("Path", "Machine")
    $user = [Environment]::GetEnvironmentVariable("Path", "User")
    $env:Path = "$machine;$user"
}

$IsArm64 = (Get-CimInstance Win32_Processor).Architecture -eq 12  # 12 = ARM64

# ---------------------------------------------------------------------------
Write-Step "Rust (rustup)"
if (Get-Command rustc -ErrorAction SilentlyContinue) {
    Write-Host "already installed: $(rustc --version)"
} else {
    $installer = "$env:TEMP\rustup-init.exe"
    Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $installer -UseBasicParsing
    & $installer -y
    Refresh-Path
    Write-Host "installed: $(rustc --version)"
}

# ---------------------------------------------------------------------------
Write-Step "Visual Studio Build Tools (C++ workload — required for the MSVC linker)"
$clExists = Get-ChildItem -Path "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC" `
    -Recurse -Filter cl.exe -ErrorAction SilentlyContinue | Select-Object -First 1
if ($clExists) {
    Write-Host "already installed: $($clExists.FullName)"
} else {
    Write-Host "downloading (this is the slow one — several GB, can take 15-40+ min)..."
    $installer = "$env:TEMP\vs_buildtools.exe"
    Invoke-WebRequest -Uri "https://aka.ms/vs/17/release/vs_buildtools.exe" -OutFile $installer -UseBasicParsing
    & $installer --quiet --wait --norestart --nocache `
        --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended
    Write-Host "Visual Studio Build Tools installed."
}

# ---------------------------------------------------------------------------
if ($IsArm64) {
    Write-Step "LLVM/clang (ARM64 host only — the 'ring' crate needs clang to assemble on aarch64-pc-windows-msvc; x64 Windows doesn't hit this, ring ships pregenerated x86_64 asm)"
    if (Get-Command clang -ErrorAction SilentlyContinue) {
        Write-Host "already installed: $(clang --version | Select-Object -First 1)"
    } else {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/llvm/llvm-project/releases/latest"
        $asset = $release.assets | Where-Object { $_.name -match "woa64\.msi$" } | Select-Object -First 1
        if (-not $asset) {
            throw "Could not find a woa64 (Windows-on-ARM64) LLVM installer in the latest release — check https://github.com/llvm/llvm-project/releases manually."
        }
        $installer = "$env:TEMP\$($asset.name)"
        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $installer -UseBasicParsing
        $log = "$env:TEMP\llvm_install.log"
        $proc = Start-Process msiexec.exe -ArgumentList "/i `"$installer`" /quiet /norestart /L*v `"$log`"" -Wait -PassThru
        if ($proc.ExitCode -ne 0) {
            throw "LLVM install failed (exit $($proc.ExitCode)) — check $log. Common cause: low disk space on C: (LLVM needs several GB free)."
        }
        $llvmBin = "C:\Program Files\LLVM\bin"
        $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
        if ($machinePath -notlike "*$llvmBin*") {
            [Environment]::SetEnvironmentVariable("Path", "$machinePath;$llvmBin", "Machine")
        }
        Refresh-Path
        Write-Host "installed: $(clang --version | Select-Object -First 1)"
    }
} else {
    Write-Step "LLVM/clang — skipped (only needed on ARM64 hosts for the 'ring' crate)"
}

# ---------------------------------------------------------------------------
Write-Step "Node.js LTS + pnpm"
if (Get-Command node -ErrorAction SilentlyContinue) {
    Write-Host "node already installed: $(node --version)"
} else {
    $index = Invoke-RestMethod -Uri "https://nodejs.org/dist/index.json"
    $lts = $index | Where-Object { $_.lts -ne $false } | Select-Object -First 1
    $msiName = "node-$($lts.version)-arm64.msi"
    if (-not $IsArm64) { $msiName = "node-$($lts.version)-x64.msi" }
    $installer = "$env:TEMP\$msiName"
    Invoke-WebRequest -Uri "https://nodejs.org/dist/$($lts.version)/$msiName" -OutFile $installer -UseBasicParsing
    Start-Process msiexec.exe -ArgumentList "/i `"$installer`" /quiet /norestart" -Wait
    Refresh-Path
    Write-Host "installed: $(node --version)"
}
if (Get-Command pnpm -ErrorAction SilentlyContinue) {
    Write-Host "pnpm already installed: $(pnpm --version)"
} else {
    npm install -g pnpm
    Refresh-Path
    Write-Host "installed: $(pnpm --version)"
}

# ---------------------------------------------------------------------------
Write-Step "Python 3.11+ and uv"
if (Get-Command python -ErrorAction SilentlyContinue) {
    Write-Host "python already installed: $(python --version)"
    Write-Host "NOTE: if this is the Microsoft Store alias (not a real install), it will silently no-op instead of erroring — verify with 'where python'."
} else {
    # Resolve the current 3.12 release from python.org rather than hardcoding
    # a version that may no longer exist.
    $ftpIndex = Invoke-WebRequest -Uri "https://www.python.org/ftp/python/" -UseBasicParsing
    $versions = [regex]::Matches($ftpIndex.Content, '3\.12\.\d+') | ForEach-Object { $_.Value } | Sort-Object { [version]$_ } -Descending
    $version = $versions | Select-Object -First 1
    $arch = if ($IsArm64) { "arm64" } else { "amd64" }
    $installer = "$env:TEMP\python-$version-$arch.exe"
    Invoke-WebRequest -Uri "https://www.python.org/ftp/python/$version/python-$version-$arch.exe" -OutFile $installer -UseBasicParsing
    Start-Process $installer -ArgumentList "/quiet InstallAllUsers=1 PrependPath=1" -Wait
    Refresh-Path
    Write-Host "installed: $(python --version)"
}
if (Get-Command uv -ErrorAction SilentlyContinue) {
    Write-Host "uv already installed: $(uv --version)"
} else {
    python -m pip install uv
    Refresh-Path
    Write-Host "installed: $(uv --version)"
}

# ---------------------------------------------------------------------------
Write-Step "PowerShell execution policy"
# Default Windows PowerShell (Restricted) refuses to run ANY .ps1 script,
# including the .ps1 shim npm generates for every globally-installed CLI
# (claude, pnpm, etc.) — typing the bare command name fails with
# "running scripts is disabled on this system" even though the .cmd/.exe
# sitting right next to it would work fine. RemoteSigned is the standard
# safe default: local/self-authored scripts run, downloaded ones still need
# a signature.
if ((Get-ExecutionPolicy -Scope LocalMachine) -eq "Restricted") {
    Set-ExecutionPolicy -Scope LocalMachine -ExecutionPolicy RemoteSigned -Force
    Write-Host "set LocalMachine execution policy to RemoteSigned"
} else {
    Write-Host "already permissive enough: $(Get-ExecutionPolicy -Scope LocalMachine)"
}

# ---------------------------------------------------------------------------
Write-Step "Claude Code CLI"
if (Get-Command claude -ErrorAction SilentlyContinue) {
    Write-Host "already installed: $(claude --version)"
} else {
    npm install -g @anthropic-ai/claude-code
    Refresh-Path
    Write-Host "installed. Run 'claude' in a NEW terminal window and follow the login prompt (opens a browser to authorize your account)."
}

# ---------------------------------------------------------------------------
Write-Step "Project dependencies"
$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    pnpm install
    Push-Location "src-python"
    try {
        uv sync
    } catch {
        Write-Host ""
        Write-Host "uv sync failed — if the error mentions 'pyaudiowpatch' and 'win_arm64', this is a KNOWN gap:" -ForegroundColor Yellow
        Write-Host "pyaudiowpatch ships wheels for win32/win_amd64 only, not win_arm64. See product_docs/WINDOWS_PARITY_AUDIT.md." -ForegroundColor Yellow
        Write-Host "Workaround to keep developing everything except live audio capture on an ARM64 machine:" -ForegroundColor Yellow
        Write-Host "    uv pip install structlog numpy -e . --no-deps" -ForegroundColor Yellow
    }
    Pop-Location
    Push-Location "src-tauri"
    cargo build
    Pop-Location
} finally {
    Pop-Location
}

Write-Step "Done"
Write-Host "Next steps:"
Write-Host "  1. Open a NEW terminal window (so PATH changes from this script take effect)."
Write-Host "  2. Run 'claude' once and log in."
Write-Host "  3. From the repo root, run: pnpm tauri dev"
