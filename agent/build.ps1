param(
    [Parameter(Mandatory = $true)]
    [string]$MonoCoreDirectory,
    [Parameter(Mandatory = $true)]
    [string]$Il2CppCoreDirectory
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$vendor = Join-Path $root "vendor"
$monoVendor = Join-Path $vendor "mono/core"
$il2CppVendor = Join-Path $vendor "il2cpp/core"

foreach ($required in @(
    (Join-Path $MonoCoreDirectory "BepInEx.dll"),
    (Join-Path $Il2CppCoreDirectory "BepInEx.Core.dll"),
    (Join-Path $Il2CppCoreDirectory "BepInEx.Unity.IL2CPP.dll")
)) {
    if (-not (Test-Path -Path $required -PathType Leaf)) {
        throw "Required BepInEx assembly was not found: $required"
    }
}

New-Item -ItemType Directory -Force -Path $monoVendor, $il2CppVendor | Out-Null
Copy-Item (Join-Path $MonoCoreDirectory "BepInEx.dll") $monoVendor -Force
Copy-Item (Join-Path $Il2CppCoreDirectory "BepInEx.Core.dll") $il2CppVendor -Force
Copy-Item (Join-Path $Il2CppCoreDirectory "BepInEx.Unity.IL2CPP.dll") $il2CppVendor -Force

foreach ($artifactDirectory in @(
    (Join-Path $root "artifacts/mono"),
    (Join-Path $root "artifacts/il2cpp")
)) {
    New-Item -ItemType Directory -Force -Path $artifactDirectory | Out-Null
    Get-ChildItem -LiteralPath $artifactDirectory -Force |
        Where-Object { $_.Name -ne "README.txt" } |
        Remove-Item -Recurse -Force
}

dotnet test (Join-Path $root "tests/GameTweaks.Agent.Core.Tests/GameTweaks.Agent.Core.Tests.csproj") --configuration Release
if ($LASTEXITCODE -ne 0) { throw "Agent core and SDK tests failed." }
dotnet build (Join-Path $root "src/GameTweaks.Agent.Mono/GameTweaks.Agent.Mono.csproj") --configuration Release
if ($LASTEXITCODE -ne 0) { throw "The BepInEx 5 Mono Agent host build failed." }
dotnet build (Join-Path $root "src/GameTweaks.Agent.IL2CPP/GameTweaks.Agent.IL2CPP.csproj") --configuration Release
if ($LASTEXITCODE -ne 0) { throw "The BepInEx 6 IL2CPP Agent host build failed." }
