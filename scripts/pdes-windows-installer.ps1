# -------------------------------
# Install BOSSAC (BOSSA CLI)
# -------------------------------

$ErrorActionPreference = "Stop"

# Configuration
$Version      = "1.9.1"
$InstallDir   = "C:\Program Files\BOSSA"
$TempDir      = "$env:TEMP\bossac_install"
$ZipFile      = "$TempDir\bossa.zip"
$DownloadUrl  = "https://github.com/shumatech/BOSSA/releases/download/$Version/bossa-$Version-x64-windows.zip"

Write-Host "Installing BOSSAC version $Version..." -ForegroundColor Cyan

# Create temp directory
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null

# Download BOSSA
Write-Host "Downloading BOSSA..."
Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipFile

# Create install directory
Write-Host "Creating install directory..."
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

# Extract
Write-Host "Extracting files..."
Expand-Archive -Path $ZipFile -DestinationPath $InstallDir -Force

# Add to PATH (system-wide)
Write-Host "Adding BOSSA to system PATH..."
$CurrentPath = [Environment]::GetEnvironmentVariable("Path", "Machine")

if ($CurrentPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable(
        "Path",
        "$CurrentPath;$InstallDir",
        "Machine"
    )
}

# Cleanup
Remove-Item -Recurse -Force $TempDir

Write-Host "BOSSAC installation completed." -ForegroundColor Green
Write-Host "You may need to restart your terminal or reboot for PATH changes to apply."

# Verify installation
Write-Host "Verifying installation..."
bossac --help

