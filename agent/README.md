# GameTweaks Agent

## Build

Install a current .NET SDK, extract the exact BepInEx 5 Mono and BepInEx 6
IL2CPP packages that the desktop installer supports, then run on Windows:

```powershell
./agent/build.ps1 `
  -MonoCoreDirectory C:\path\to\bepinex5\BepInEx\core `
  -Il2CppCoreDirectory C:\path\to\bepinex6\BepInEx\core
```

The build script fails if the required host assemblies are absent, runs the SDK
tests, and writes the two bundle variants to `agent/artifacts/mono` and
`agent/artifacts/il2cpp`. The Tauri bundle includes those directories as app
resources. Never commit extracted BepInEx dependencies under `agent/vendor`.

The Agent is a shared BepInEx plugin. It exposes only typed mod metadata and
configuration values to the GameTweaks desktop app over the authenticated local
Named Pipe protocol documented in `PROTOCOL.md`.

Projects:

- `GameTweaks.Agent.Abstractions`: dependency-free SDK contract published on
  NuGet for mod authors.
- `GameTweaks.Agent.Core`: registry, validation, reconnect and Named Pipe protocol.
- `GameTweaks.Agent.Mono`: thin BepInEx 5 Unity Mono host.
- `GameTweaks.Agent.IL2CPP`: thin BepInEx 6 Unity IL2CPP host.
- `UnityEngine.Reference`: compile-only `MonoBehaviour` identity needed because
  the generic BepInEx 5 package intentionally does not ship a game's Unity DLL.

The host projects deliberately reference assemblies from official unpacked
BepInEx distributions in `vendor/mono/core` and `vendor/il2cpp/core`. Do not
commit those binaries. Build outputs copied to `artifacts/mono` and
`artifacts/il2cpp` are bundled by the desktop release pipeline.

CI and release builds run `build-release.ps1`. It accepts only the latest
official stable BepInEx 5.4 x64 GitHub asset with its published SHA-256 digest
and the latest exact IL2CPP x64 artifact from `builds.bepinex.dev`, then calls
the same strict host build above. The resulting architecture-neutral Agent
assemblies are uploaded once and injected into every Tauri bundle job.

The current workspace does not include vendor binaries. The SDK and Core can be
built and tested independently:

```powershell
dotnet test agent/tests/GameTweaks.Agent.Core.Tests/GameTweaks.Agent.Core.Tests.csproj
```

## Local mod development

Enable **Developer Mode** in the desktop app settings. Local `pnpm dev` builds
force it on. After BepInEx is installed and the game is closed, open the game
details and select **Install / repair dev agent**. GameTweaks creates the
per-game marker and authentication secret and installs the matching bundled
Agent safely.

Copy a development plugin to
`BepInEx/plugins/GameTweaks/<mod-id>/`. A plugin that registers through
`GameTweaks.Agent.Abstractions` appears as `External` while the game and desktop
app are connected. External development plugins can expose live configuration,
but GameTweaks does not install, update, uninstall, or mark them Official.

## Mod SDK usage

Install `GameTweaks.Agent.Abstractions` from NuGet and follow the complete
[SDK guide](SDK.md). The guide covers the required hard BepInEx dependency,
lifecycle-safe registration, supported value types, deferred settings, catalog
matching, and release packaging. A buildable BepInEx 5 Mono plugin is available
under [`examples/GameTweaks.Agent.Example.Mono`](examples/GameTweaks.Agent.Example.Mono).

## Publish the SDK

NuGet publication uses Trusted Publishing from `.github/workflows/release.yml`;
no long-lived API key is stored. Configure the repository secret `NUGET_USER`
with the individual nuget.org profile name that manages the policy. The policy's
workflow filename is `release.yml` and its environment remains empty.

Update all Agent version constants together, then push the dedicated tag
`agent-sdk-v<package-version>`. For example, `agent-sdk-v0.1.0` publishes
package version `0.1.0`. The release job rejects a tag that differs from the
package, host, Core, or desktop Agent version, runs the tests, verifies the
compile-only package and example output, and only then requests a temporary
NuGet credential.
