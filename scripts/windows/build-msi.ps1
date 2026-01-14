param(
    [string]$PayloadDir = "dist\PDES",
    [string]$OutputMsi = "dist\PDES.msi",
    [string]$ProductWxs = "installer\product.wxs",
    [string]$PayloadWxs = "installer\payload.wxs"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$tools = @("heat", "candle", "light")
foreach ($tool in $tools) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "Missing WiX tool '$tool'. Add WiX 3.x bin to PATH (e.g., `"C:\Program Files (x86)\WiX Toolset v3.11\bin`")."
    }
}

if (-not (Test-Path $PayloadDir)) {
    throw "Payload directory '$PayloadDir' not found. Run build_dist first."
}

# Ensure output directory exists
$null = New-Item -ItemType Directory -Force -Path (Split-Path $OutputMsi) | Out-Null

# Harvest current payload
heat dir $PayloadDir `
    -dr INSTALLDIR `
    -cg PDESPayload `
    -var var.SourceDir `
    -out $PayloadWxs `
    -srd `
    -sfrag `
    -ag

# Mark all harvested components as 64-bit to satisfy ICE80 when installing to ProgramFiles64Folder
(Get-Content $PayloadWxs -Raw) -replace '<Component Id="', '<Component Win64="yes" Id="' | Set-Content $PayloadWxs

# Compile + link
candle -dSourceDir="$PayloadDir" $ProductWxs $PayloadWxs
light -ext WixUIExtension -ext WixUtilExtension -o $OutputMsi product.wixobj payload.wixobj

Write-Host "MSI created at $OutputMsi"
