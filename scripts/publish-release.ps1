param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [string]$Notes = "inspiration box $Version"
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$bundleDir = Join-Path $repoRoot "src-tauri\target\release\bundle\nsis"
$sourceInstaller = Join-Path $bundleDir "inspiration box_${Version}_x64-setup.exe"
$sourceSignature = "$sourceInstaller.sig"
$assetName = "inspiration-box_${Version}_x64-setup.exe"
$assetPath = Join-Path $bundleDir $assetName
$signaturePath = "$assetPath.sig"
$latestJsonPath = Join-Path $bundleDir "latest.json"

if (-not (Test-Path -LiteralPath $sourceInstaller) -or
    -not (Test-Path -LiteralPath $sourceSignature)) {
    throw "缺少签名构建产物，请先使用签名环境变量运行 npm run build。"
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
$latest | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 -LiteralPath $latestJsonPath

gh release create "v$Version" $assetPath $signaturePath $latestJsonPath `
    --title "inspiration box v$Version" `
    --notes $Notes
