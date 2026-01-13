# ============================================================
# PDES Windows Installer (Admin Required)
# ============================================================

$ErrorActionPreference = "Stop"

# ------------------------------------------------------------
# Elevation check
# ------------------------------------------------------------
if (-not ([Security.Principal.WindowsPrincipal]
    [Security.Principal.WindowsIdentity]::GetCurrent()
).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {

    Write-Host "Re-launching installer as Administrator..."
    Start-Process powershell `
        -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`"" `
        -Verb RunAs
    exit
}

# ------------------------------------------------------------
# Configuration
# ------------------------------------------------------------
$AppName        = "PDES"
$InstallDir    = "C:\Program Files\PDES"
$ToolsDir      = "$InstallDir\tools"

$BossaVersion  = "1.9.1"
$BossaZipUrl   = "https://github.com/shumatech/BOSSA/releases/download/$BossaVersion/bossa-$BossaVersion-x64-windows.zip"
$TempDir       = "$env:TEMP\pdes_install"
$BossaZip      = "$TempDir\bossa.zip"

$BundledZadig  = "$PSScriptRoot\zadig.exe"   # EXPECTED TO BE BUNDLED
$BossaExe      = "$ToolsDir\bossac.exe"
$AppExe        = "$InstallDir\pdes.exe"

# ------------------------------------------------------------
# Pre-flight checks
# ------------------------------------------------------------
if (-not [Environment]::Is64BitOperatingSystem) {
    throw "32-bit Windows is not supported."
}

Write-Host "Installing $AppName..." -ForegroundColor Cyan

# ------------------------------------------------------------
# Prepare directories
# ------------------------------------------------------------
New-Item -ItemType Directory -Force -Path $TempDir     | Out-Null
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path $ToolsDir   | Out-Null

# ------------------------------------------------------------
# Copy application files (MSI/Inno should place them first)
# ------------------------------------------------------------
if (-not (Test-Path $AppExe)) {
    throw "pdes.exe not found in $InstallDir. Installer packaging error."
}

# ------------------------------------------------------------
# Install BOSSA privately
# ------------------------------------------------------------
if (-not (Test-Path $BossaExe)) {

    Write-Host "Downloading BOSSA..." -ForegroundColor Cyan
    Invoke-WebRequest -Uri $BossaZipUrl -OutFile $BossaZip

    Write-Host "Extracting BOSSA..."
    Expand-Archive -Path $BossaZip -DestinationPath $ToolsDir -Force
}

Write-Host "Verifying BOSSA installation..."
& $BossaExe --help | Out-Null

# ------------------------------------------------------------
# Device detection
# ------------------------------------------------------------
Write-Host "Detecting connected devices..."

$Devices = Get-PnpDevice -PresentOnly |
    Where-Object {
        $_.Class -in @("USB", "Ports") -and
        $_.FriendlyName -match "Arduino|CDC|BOSSA|SAM|Atmel"
    }

if ($Devices.Count -eq 0) {
    Write-Warning "No compatible device detected."
    Write-Warning "Please connect your device before using PDES."
} else {
    Write-Host "Detected device(s):"
    $Devices | Format-Table FriendlyName, Status
}

# ------------------------------------------------------------
# Driver check & Zadig
# ------------------------------------------------------------
$NeedsDriver = $Devices |
    Where-Object { $_.Status -ne "OK" }

if ($NeedsDriver.Count -gt 0) {

    if (-not (Test-Path $BundledZadig)) {
        Write-Warning "Zadig not bundled. Driver installation skipped."
    } else {
        Write-Warning "USB driver not installed correctly."
        Write-Host "Launching Zadig for driver installation..." -ForegroundColor Yellow

        Start-Process `
            -FilePath $BundledZadig `
            -Verb RunAs `
            -Wait

        Write-Host "Zadig closed. Driver installation may require device replug."
    }
}

# ------------------------------------------------------------
# Create shortcuts
# ------------------------------------------------------------
Write-Host "Creating shortcuts..."

$Shell = New-Object -ComObject WScript.Shell

# Desktop shortcut
$DesktopShortcut = "$([Environment]::GetFolderPath('Desktop'))\PDES.lnk"
$s = $Shell.CreateShortcut($DesktopShortcut)
$s.TargetPath = $AppExe
$s.WorkingDirectory = $InstallDir
$s.IconLocation = "$AppExe,0"
$s.Save()

# Start Menu shortcut
$StartMenuDir = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs"
$StartShortcut = "$StartMenuDir\PDES.lnk"
$s = $Shell.CreateShortcut($StartShortcut)
$s.TargetPath = $AppExe
$s.WorkingDirectory = $InstallDir
$s.IconLocation = "$AppExe,0"
$s.Save()

# ------------------------------------------------------------
# Cleanup
# ------------------------------------------------------------
Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue

# ------------------------------------------------------------
# Done
# ------------------------------------------------------------
Write-Host ""
Write-Host "$AppName installation completed successfully." -ForegroundColor Green
Write-Host "BOSSA is installed privately for PDES."
Write-Host "If your device is not detected, reconnect it and ensure the driver is installed."

