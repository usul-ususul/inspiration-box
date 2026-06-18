param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [string]$Notes = "ahhhh mmt $Version"
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$bundleDir = Join-Path $repoRoot "src-tauri\target\release\bundle\nsis"
$sourceInstaller = Join-Path $bundleDir "ahhhh mmt_${Version}_x64-setup.exe"
$sourceSignature = "$sourceInstaller.sig"
$assetName = "ahhhh-mmt_${Version}_x64-setup.exe"
$assetPath = Join-Path $bundleDir $assetName
$signaturePath = "$assetPath.sig"
$latestJsonPath = Join-Path $bundleDir "latest.json"

if (-not (Test-Path -LiteralPath $sourceInstaller) -or
    -not (Test-Path -LiteralPath $sourceSignature)) {
    throw "Signed build artifacts are missing. Run npm run build with signing variables first."
}

Copy-Item -LiteralPath $sourceInstaller -Destination $assetPath -Force
Copy-Item -LiteralPath $sourceSignature -Destination $signaturePath -Force
$signature = (Get-Content -Raw -LiteralPath $sourceSignature).Trim()
$downloadUrl = "https://github.com/usul-ususul/inspiration-box/releases/download/v$Version/$assetName"

$latest = [ordered]@{
    version = $Version
    notes = $Notes
    pub_date = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = [ordered]@{
        "windows-x86_64" = [ordered]@{
            signature = $signature
            url = $downloadUrl
        }
    }
}
$latestJson = $latest | ConvertTo-Json -Depth 5
[System.IO.File]::WriteAllText(
    $latestJsonPath,
    $latestJson,
    [System.Text.UTF8Encoding]::new($false)
)

$releaseExists = $false
try {
    gh release view "v$Version" 2>$null | Out-Null
    $releaseExists = $LASTEXITCODE -eq 0
} catch {
    $releaseExists = $false
}

if ($releaseExists) {
    gh release upload "v$Version" $assetPath $signaturePath $latestJsonPath --clobber
    gh release edit "v$Version" --title "ahhhh mmt v$Version" --notes $Notes
} else {
    gh release create "v$Version" $assetPath $signaturePath $latestJsonPath `
        --title "ahhhh mmt v$Version" `
        --notes $Notes
}
