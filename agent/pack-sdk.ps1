$ErrorActionPreference = "Stop"
$project = Join-Path $PSScriptRoot "src/GameTweaks.Agent.Abstractions/GameTweaks.Agent.Abstractions.csproj"
$sample = Join-Path $PSScriptRoot "examples/GameTweaks.Agent.Example.Mono/GameTweaks.Agent.Example.Mono.csproj"
$sampleDirectory = Split-Path -Parent $sample
$nugetConfig = Join-Path $PSScriptRoot "NuGet.CI.config"
$nuspec = Join-Path $PSScriptRoot "src/GameTweaks.Agent.Abstractions/GameTweaks.Agent.Abstractions.nuspec"
$output = Join-Path $PSScriptRoot "artifacts/nuget"
$packageCache = Join-Path $PSScriptRoot "artifacts/sdk-packages"

foreach ($path in @($output, $packageCache)) {
    if (Test-Path -LiteralPath $path) {
        Remove-Item -LiteralPath $path -Recurse -Force
    }
}
New-Item -ItemType Directory -Path $output | Out-Null

[xml]$nuspecDocument = Get-Content -LiteralPath $nuspec -Raw
$namespace = New-Object System.Xml.XmlNamespaceManager($nuspecDocument.NameTable)
$namespace.AddNamespace("n", $nuspecDocument.DocumentElement.NamespaceURI)
if ($nuspecDocument.SelectSingleNode("/n:package/n:metadata/n:references", $namespace)) {
    throw "A ref-only PackageReference package must not declare legacy nuspec references."
}
$referenceFiles = @($nuspecDocument.SelectNodes("/n:package/n:files/n:file", $namespace))
$referenceFile = $referenceFiles | Where-Object {
    ($_.target -replace "\\", "/").TrimEnd("/") -eq "ref/netstandard2.0" -and
    ($_.src -replace "\\", "/").EndsWith(
        "/GameTweaks.Agent.Abstractions.dll",
        [System.StringComparison]::OrdinalIgnoreCase)
}
if (@($referenceFile).Count -ne 1) {
    throw "The Agent SDK nuspec must place its assembly under ref/netstandard2.0."
}

foreach ($directory in @("bin", "obj")) {
    $path = Join-Path $sampleDirectory $directory
    if (Test-Path -LiteralPath $path) {
        Remove-Item -LiteralPath $path -Recurse -Force
    }
}

$packArguments = @(
    "pack",
    $project,
    "--configuration", "Release",
    "--output", $output,
    "-p:ContinuousIntegrationBuild=true"
)
if ($env:GITHUB_SHA) {
    $packArguments += "-p:RepositoryCommit=$($env:GITHUB_SHA)"
}

& dotnet @packArguments
if ($LASTEXITCODE -ne 0) {
    throw "The Agent SDK package build failed."
}

$packages = @(Get-ChildItem -LiteralPath $output -Filter "*.nupkg" -File)
if ($packages.Count -ne 1) {
    throw "Expected exactly one Agent SDK package."
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($packages[0].FullName)
try {
    $entries = @($archive.Entries | ForEach-Object { $_.FullName })
    $sdkAssemblies = @($entries | Where-Object {
        $_.EndsWith("/GameTweaks.Agent.Abstractions.dll", [System.StringComparison]::OrdinalIgnoreCase)
    })
    if ($sdkAssemblies.Count -ne 1 -or
        $sdkAssemblies[0] -ne "ref/netstandard2.0/GameTweaks.Agent.Abstractions.dll") {
        throw "The Agent SDK package must contain exactly one compile-time reference assembly."
    }
    if ($entries -notcontains "SDK.md") {
        throw "The Agent SDK package is missing its README."
    }
    if ($entries | Where-Object { $_ -like "lib/*" -or $_ -like "runtimes/*" }) {
        throw "The Agent SDK package must not contain runtime assemblies."
    }
}
finally {
    $archive.Dispose()
}

& dotnet restore $sample `
    --force-evaluate `
    --no-cache `
    --configfile $nugetConfig `
    --packages $packageCache
if ($LASTEXITCODE -ne 0) {
    throw "The Agent SDK example restore failed."
}

& dotnet build $sample --configuration Release --no-restore
if ($LASTEXITCODE -ne 0) {
    throw "The Agent SDK example build failed."
}

$sampleOutput = Join-Path $sampleDirectory "bin/Release/netstandard2.0"
$plugin = Join-Path $sampleOutput "Example.Accessibility.dll"
if (-not (Test-Path -LiteralPath $plugin -PathType Leaf)) {
    throw "The Agent SDK example did not produce its plugin DLL."
}

$outputAssemblies = @(Get-ChildItem -LiteralPath $sampleOutput -Filter "*.dll" -File -Recurse)
if ($outputAssemblies.Count -ne 1 -or $outputAssemblies[0].FullName -ne $plugin) {
    throw "The Agent SDK example output must contain only Example.Accessibility.dll."
}

Remove-Item -LiteralPath $packageCache -Recurse -Force
Write-Host "Validated $($packages[0].FullName) and the compile-only example output."
