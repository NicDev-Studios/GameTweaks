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

- `GameTweaks.Agent.Abstractions`: dependency-free SDK contract for mod authors.
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

## Mod SDK usage

Register after `GameTweaksApi.Available` fires (or immediately when
`TryGetAgent` succeeds). A BepInEx `ConfigEntry<T>` can be exposed with the
dependency-free delegate binding:

```csharp
agent.RegisterMod(new(
    "example.accessibility", "1.0.0",
    new("Accessibility"), new("Accessibility controls")));

agent.RegisterSetting("example.accessibility", new(
    "highContrast", "Accessibility", "HighContrast", new("High contrast"),
    SettingKind.Boolean, configEntry.DefaultValue, SettingApplyMode.Live,
    SettingDisplay.Switch),
    new DelegateSettingBinding<bool>(
        () => configEntry.Value,
        value => { configEntry.Value = value; return SettingChangeResult.Success(); }));
```

For `RestartRequired` and `NextLaunch`, provide the binding's optional `store`
callback. The Agent deliberately rejects a deferred change when the binding
cannot persist it without applying it to the current session. Call
`NotifyValueChanged()` when the mod changes a live value itself.
