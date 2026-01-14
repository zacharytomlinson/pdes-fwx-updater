$ErrorActionPreference = "Stop"

$InstallDir = "C:\Program Files\PDES"

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host "Re-launching uninstaller as Administrator..."
    Start-Process powershell `
        -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`"" `
        -Verb RunAs
    exit
}

Write-Host "Removing PDES..."

# Remove shortcuts
Remove-Item "$([Environment]::GetFolderPath('Desktop'))\PDES.lnk" -ErrorAction SilentlyContinue
Remove-Item "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\PDES.lnk" -ErrorAction SilentlyContinue

# Remove install directory
Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue

Write-Host "PDES uninstalled successfully."
