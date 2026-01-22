param(
    [Parameter(Mandatory=$true)][string]$Origin,
    [Parameter(Mandatory=$true)][string]$WriterToken,
    [string]$ApiBase = "",
    [string]$AppSlug = "pdes-fw-x-updater",
    [string]$Target = "windows:x86_64",
    [int]$ExpiresInSeconds = 3600,
    [int]$MaxUses = 10
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($MaxUses -lt 2) { $MaxUses = 2 }

$Origin = $Origin.TrimEnd("/")
if ([string]::IsNullOrWhiteSpace($ApiBase)) {
    $ApiBase = "$Origin/api"
}
$ApiBase = $ApiBase.TrimEnd("/")

$body = @{
    app_slug = $AppSlug
    target = $Target
    expires_in_seconds = $ExpiresInSeconds
    max_uses = $MaxUses
} | ConvertTo-Json

$headers = @{
    Authorization = "Bearer $WriterToken"
}

$resp = Invoke-RestMethod -Method Post -Uri "$ApiBase/bootstrap/mint" -Headers $headers -ContentType "application/json" -Body $body

$pkgUrl = $null
if ($resp.bootstrap_pkg_url) { $pkgUrl = $resp.bootstrap_pkg_url }
elseif ($resp.bootstrap_token) { $pkgUrl = "$Origin/bootstrap/$($resp.bootstrap_token)/pkg" }

if (-not $pkgUrl) {
    throw "No bootstrap_pkg_url/bootstrap_token in mint response."
}

Write-Output $pkgUrl
