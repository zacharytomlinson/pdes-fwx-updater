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

# Post-process payload to satisfy ICE38/ICE64 for per-user installs (LocalAppDataFolder)
[xml]$payloadXml = Get-Content $PayloadWxs
$nsUri = "http://schemas.microsoft.com/wix/2006/wi"
$nsMgr = New-Object System.Xml.XmlNamespaceManager($payloadXml.NameTable)
$nsMgr.AddNamespace("w", $nsUri)

$componentGroup = $payloadXml.SelectSingleNode("//w:ComponentGroup[@Id='PDESPayload']", $nsMgr)
if (-not $componentGroup) {
    throw "ComponentGroup 'PDESPayload' not found in $PayloadWxs"
}

# Ensure every component uses an HKCU registry value as its KeyPath (fixes ICE38)
foreach ($component in $payloadXml.SelectNodes("//w:Component", $nsMgr)) {
    $componentId = $component.GetAttribute("Id")

    foreach ($file in $component.SelectNodes("./w:File[@KeyPath='yes']", $nsMgr)) {
        $file.RemoveAttribute("KeyPath")
    }

    if (-not $component.SelectSingleNode("./w:RegistryValue[@KeyPath='yes']", $nsMgr)) {
        $reg = $payloadXml.CreateElement("RegistryValue", $nsUri)
        $reg.SetAttribute("Root", "HKCU")
        $reg.SetAttribute("Key", "Software\\PDES\\Installer\\Components")
        $reg.SetAttribute("Name", $componentId)
        $reg.SetAttribute("Type", "integer")
        $reg.SetAttribute("Value", "1")
        $reg.SetAttribute("KeyPath", "yes")
        $component.PrependChild($reg) | Out-Null
    }

    if (-not $component.GetAttribute("Win64")) {
        $component.SetAttribute("Win64", "yes")
    }
}

# Add cleanup components with RemoveFile entries for every DirectoryRef (fixes ICE64)
$cleanupIds = @()
foreach ($dirRef in $payloadXml.SelectNodes("//w:DirectoryRef", $nsMgr)) {
    $dirId = $dirRef.GetAttribute("Id")
    $cleanupId = "Cleanup_$dirId"

    if (-not $payloadXml.SelectSingleNode("//w:Component[@Id='$cleanupId']", $nsMgr)) {
        $cleanup = $payloadXml.CreateElement("Component", $nsUri)
        $cleanup.SetAttribute("Id", $cleanupId)
        $cleanup.SetAttribute("Guid", "*")
        $cleanup.SetAttribute("Win64", "yes")

        $reg = $payloadXml.CreateElement("RegistryValue", $nsUri)
        $reg.SetAttribute("Root", "HKCU")
        $reg.SetAttribute("Key", "Software\\PDES\\Installer\\Cleanup")
        $reg.SetAttribute("Name", $cleanupId)
        $reg.SetAttribute("Type", "integer")
        $reg.SetAttribute("Value", "1")
        $reg.SetAttribute("KeyPath", "yes")
        $cleanup.AppendChild($reg) | Out-Null

        $removeFiles = $payloadXml.CreateElement("RemoveFile", $nsUri)
        $removeFiles.SetAttribute("Id", "Remove_$dirId")
        $removeFiles.SetAttribute("Name", "*")
        $removeFiles.SetAttribute("On", "uninstall")
        $removeFiles.SetAttribute("Directory", $dirId)
        $cleanup.AppendChild($removeFiles) | Out-Null

        $removeFolder = $payloadXml.CreateElement("RemoveFolder", $nsUri)
        $removeFolder.SetAttribute("Id", "RemoveFolder_$dirId")
        $removeFolder.SetAttribute("On", "uninstall")
        $removeFolder.SetAttribute("Directory", $dirId)
        $cleanup.AppendChild($removeFolder) | Out-Null

        $dirRef.AppendChild($cleanup) | Out-Null
        $cleanupIds += $cleanupId
    }
}

foreach ($cleanupId in $cleanupIds) {
    if (-not $componentGroup.SelectSingleNode("./w:ComponentRef[@Id='$cleanupId']", $nsMgr)) {
        $compRef = $payloadXml.CreateElement("ComponentRef", $nsUri)
        $compRef.SetAttribute("Id", $cleanupId)
        $componentGroup.AppendChild($compRef) | Out-Null
    }
}

$payloadXml.Save($PayloadWxs)

# Compile + link
candle -dSourceDir="$PayloadDir" $ProductWxs $PayloadWxs
light -ext WixUIExtension -ext WixUtilExtension -o $OutputMsi product.wixobj payload.wixobj

Write-Host "MSI created at $OutputMsi"
