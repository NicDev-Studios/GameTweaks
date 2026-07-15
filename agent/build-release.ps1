$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ("gametweaks-agent-" + [Guid]::NewGuid().ToString("N"))
$headers = @{
    Accept = "application/vnd.github+json"
    "User-Agent" = "GameTweaks-Agent-Build"
    "X-GitHub-Api-Version" = "2022-11-28"
}

function Get-SingleCoreDirectory {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$RequiredFile
    )
    $matches = @(Get-ChildItem -Path $Root -Filter $RequiredFile -File -Recurse)
    if ($matches.Count -ne 1) {
        throw "Expected exactly one $RequiredFile in the official package."
    }
    return $matches[0].Directory.FullName
}

try {
    New-Item -ItemType Directory -Force -Path $temporary | Out-Null

    $release = Invoke-RestMethod `
        -Uri "https://api.github.com/repos/BepInEx/BepInEx/releases/latest" `
        -Headers $headers
    if ($release.draft -or $release.prerelease -or $release.tag_name -notmatch '^v5\.4\.\d+(?:\.\d+)?$') {
        throw "The latest official BepInEx release is not a stable v5.4 release."
    }
    $monoAssets = @($release.assets | Where-Object {
        $_.name -match '^BepInEx_win_x64_5\.4\.\d+(?:\.\d+)?\.zip$'
    })
    if ($monoAssets.Count -ne 1) {
        throw "Expected exactly one official BepInEx 5 x64 asset."
    }
    $monoAsset = $monoAssets[0]
    if ($monoAsset.browser_download_url -notmatch '^https://github\.com/BepInEx/BepInEx/releases/download/') {
        throw "The BepInEx 5 asset URL had an unexpected origin."
    }
    if ($monoAsset.digest -notmatch '^sha256:([0-9a-fA-F]{64})$') {
        throw "The BepInEx 5 asset did not publish a SHA-256 digest."
    }
    $monoDigest = $Matches[1].ToLowerInvariant()
    $monoArchive = Join-Path $temporary "mono.zip"
    Invoke-WebRequest -Uri $monoAsset.browser_download_url -Headers $headers -OutFile $monoArchive
    if ((Get-Item $monoArchive).Length -gt 268435456) {
        throw "The BepInEx 5 package exceeded the compressed size limit."
    }
    if ((Get-FileHash $monoArchive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $monoDigest) {
        throw "The BepInEx 5 package digest did not match GitHub metadata."
    }
    $monoRoot = Join-Path $temporary "mono"
    Expand-Archive -LiteralPath $monoArchive -DestinationPath $monoRoot
    $monoCore = Get-SingleCoreDirectory -Root $monoRoot -RequiredFile "BepInEx.dll"

    $buildsRoot = [Uri]"https://builds.bepinex.dev/projects/bepinex_be"
    $buildsHtml = (Invoke-WebRequest -Uri $buildsRoot.AbsoluteUri).Content
    $pattern = 'href="(?<path>/projects/bepinex_be/(?<build>\d+)/BepInEx-Unity\.IL2CPP-win-x64-6\.0\.0-be\.\k<build>%2B[0-9a-fA-F]{7,40}\.zip)"'
    $matches = @([regex]::Matches($buildsHtml, $pattern, [System.Text.RegularExpressions.RegexOptions]::IgnoreCase))
    if ($matches.Count -eq 0) {
        throw "No supported official BepInEx IL2CPP artifact was found."
    }
    $latest = $matches | Sort-Object { [int64]$_.Groups['build'].Value } -Descending | Select-Object -First 1
    $il2CppUri = [Uri]::new($buildsRoot, $latest.Groups['path'].Value)
    if ($il2CppUri.Scheme -ne "https" -or $il2CppUri.Host -ne "builds.bepinex.dev") {
        throw "The BepInEx IL2CPP asset URL had an unexpected origin."
    }
    $il2CppArchive = Join-Path $temporary "il2cpp.zip"
    Invoke-WebRequest -Uri $il2CppUri.AbsoluteUri -OutFile $il2CppArchive -MaximumRedirection 5
    if ((Get-Item $il2CppArchive).Length -gt 268435456) {
        throw "The BepInEx IL2CPP package exceeded the compressed size limit."
    }
    $il2CppRoot = Join-Path $temporary "il2cpp"
    Expand-Archive -LiteralPath $il2CppArchive -DestinationPath $il2CppRoot
    $il2CppCore = Get-SingleCoreDirectory -Root $il2CppRoot -RequiredFile "BepInEx.Unity.IL2CPP.dll"
    if (-not (Test-Path (Join-Path $il2CppCore "BepInEx.Core.dll") -PathType Leaf)) {
        throw "The official IL2CPP package did not contain BepInEx.Core.dll beside its Unity host."
    }

    & (Join-Path $root "build.ps1") `
        -MonoCoreDirectory $monoCore `
        -Il2CppCoreDirectory $il2CppCore
}
finally {
    if (Test-Path $temporary) {
        Remove-Item -LiteralPath $temporary -Recurse -Force
    }
}
