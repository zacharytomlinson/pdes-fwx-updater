$ErrorActionPreference = "Stop"

$InstallDir = "C:\Program Files\PDES"

Write-Host "Removing PDES..."

# Remove shortcuts
Remove-Item "$([Environment]::GetFolderPath('Desktop'))\PDES.lnk" -ErrorAction SilentlyContinue
Remove-Item "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\PDES.lnk" -ErrorAction SilentlyContinue

# Remove install directory
Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue

Write-Host "PDES uninstalled successfully."
