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
# Utility: deterministic GUID from name (RFC 4122 version 5)
function New-StableGuid {
    param(
        [Parameter(Mandatory=$true)][Guid]$Namespace,
        [Parameter(Mandatory=$true)][string]$Name
    )

    $nsBytes = $Namespace.ToByteArray()
    $nameBytes = [System.Text.Encoding]::UTF8.GetBytes($Name)
    $data = New-Object byte[] ($nsBytes.Length + $nameBytes.Length)
    [Array]::Copy($nsBytes, 0, $data, 0, $nsBytes.Length)
    [Array]::Copy($nameBytes, 0, $data, $nsBytes.Length, $nameBytes.Length)

    $hash = [System.Security.Cryptography.SHA1]::Create().ComputeHash($data)
    $guidBytes = New-Object byte[] 16
    for ($i = 0; $i -lt 16; $i++) {
        $guidBytes[$i] = $hash[$i]
    }

    # version 5 + RFC 4122 variant bits
    $guidBytes[6] = ($guidBytes[6] -band 0x0F) -bor 0x50
    $guidBytes[8] = ($guidBytes[8] -band 0x3F) -bor 0x80

    return ([Guid]$guidBytes).ToString("D")
}

[xml]$payloadXml = Get-Content $PayloadWxs
$nsUri = "http://schemas.microsoft.com/wix/2006/wi"
$nsMgr = New-Object System.Xml.XmlNamespaceManager($payloadXml.NameTable)
$nsMgr.AddNamespace("w", $nsUri)
$namespaceGuid = [Guid]"3e06b236-01e3-4dce-b7ea-4c4d249f698a"

$componentGroup = $payloadXml.SelectSingleNode("//w:ComponentGroup[@Id='PDESPayload']", $nsMgr)
if (-not $componentGroup) {
    throw "ComponentGroup 'PDESPayload' not found in $PayloadWxs"
}

# Ensure every component uses an HKCU registry value as its KeyPath (fixes ICE38)
foreach ($component in $payloadXml.SelectNodes("//w:Component", $nsMgr)) {
    $componentId = $component.GetAttribute("Id")
    $component.SetAttribute("Guid", (New-StableGuid -Namespace $namespaceGuid -Name $componentId))

    foreach ($file in $component.SelectNodes("./w:File[@KeyPath='yes']", $nsMgr)) {
        $file.RemoveAttribute("KeyPath")
    }

    if (-not $component.SelectSingleNode("./w:RegistryValue[@KeyPath='yes']", $nsMgr)) {
        $reg = $payloadXml.CreateElement("RegistryValue", $nsUri)
        $reg.SetAttribute("Root", "HKCU")
        $reg.SetAttribute("Key", "Software\PDES\Installer\Components")
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

# Add cleanup components with RemoveFile entries for every harvested directory (fixes ICE64)
$cleanupIds = @()
foreach ($dir in $payloadXml.SelectNodes("//w:DirectoryRef[@Id='INSTALLDIR']//w:Directory", $nsMgr)) {
    $dirId = $dir.GetAttribute("Id")
    $cleanupId = "Cleanup_$dirId"

    if (-not $payloadXml.SelectSingleNode("//w:Component[@Id='$cleanupId']", $nsMgr)) {
        $cleanup = $payloadXml.CreateElement("Component", $nsUri)
        $cleanup.SetAttribute("Id", $cleanupId)
        $cleanup.SetAttribute("Guid", (New-StableGuid -Namespace $namespaceGuid -Name $cleanupId))
        $cleanup.SetAttribute("Win64", "yes")

        $reg = $payloadXml.CreateElement("RegistryValue", $nsUri)
        $reg.SetAttribute("Root", "HKCU")
        $reg.SetAttribute("Key", "Software\PDES\Installer\Cleanup")
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

        $dir.AppendChild($cleanup) | Out-Null
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
