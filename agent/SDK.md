# GameTweaks Agent SDK

`GameTweaks.Agent.Abstractions` is the compile-time contract used by BepInEx
mods that expose their existing settings to the GameTweaks desktop app. The
desktop app installs and owns the shared Agent runtime. A mod must never bundle
its own copy of an Agent assembly.

The current SDK and Agent versions are both `0.1.0`.

## Install the SDK

Add the package to the mod project as a private, compile-only dependency:

```xml
<PackageReference Include="GameTweaks.Agent.Abstractions" Version="0.1.0">
  <IncludeAssets>compile</IncludeAssets>
  <PrivateAssets>all</PrivateAssets>
</PackageReference>
```

The package contains a reference assembly under `ref/` and deliberately has no
runtime asset. The GameTweaks Agent installed beside BepInEx supplies the real
assembly when the game starts.

An Agent-integrated mod has a hard BepInEx dependency on the oldest Agent
version it supports:

```csharp
[BepInDependency("dev.gametweaks.agent", "0.1.0")]
```

Do not mark this dependency as soft. A mod with a direct SDK reference cannot
load safely without the shared runtime assembly.

## Register safely

Bind the mod's configuration first, then subscribe once to
`GameTweaksApi.Available`. The event immediately supplies the current Agent
when it is already available and fires again if the Agent is replaced. Remove
the handler and unregister the mod during plugin shutdown.

```csharp
private const string ModId = "example.accessibility";
private IGameTweaksAgent? _agent;

private void Awake()
{
    // Bind the BepInEx ConfigEntry<T> values before subscribing.
    GameTweaksApi.Available += RegisterWithAgent;
}

private void RegisterWithAgent(IGameTweaksAgent agent)
{
    if (ReferenceEquals(_agent, agent))
        return;

    _agent?.UnregisterMod(ModId);

    agent.RegisterMod(new ModRegistration(
        ModId,
        "1.0.0",
        new LocalizedText("Accessibility Example"),
        new LocalizedText("Accessibility controls")));

    // RegisterSetting calls follow here.
    _agent = agent;
}

private void OnDestroy()
{
    GameTweaksApi.Available -= RegisterWithAgent;
    _agent?.UnregisterMod(ModId);
    _agent = null;
}
```

Registration can fail when metadata is invalid or another loaded plugin already
owns the same mod ID. Production plugins should catch registration errors, log
them through BepInEx, and call `UnregisterMod` to roll back a partial
registration. The
[complete Mono example](https://github.com/NicDev-Studios/GameTweaks/tree/main/agent/examples/GameTweaks.Agent.Example.Mono)
implements that lifecycle, including change notifications and rollback.

For an IL2CPP mod, use the same SDK package, hard dependency, metadata, and
bindings. Subscribe from the BepInEx 6 IL2CPP plugin's `Load()` method and
unsubscribe plus unregister from `Unload()`. Keep the BepInEx and interop
references matched to the exact game/runtime targeted by the catalog entry;
the included project is intentionally a BepInEx 5 Mono example.

## Expose a setting

For ordinary BepInEx values, `DelegateSettingBinding<T>` connects a getter and
setter without giving the Agent access to the config file or other mod state:

```csharp
var binding = new DelegateSettingBinding<bool>(
    () => highContrast.Value,
    value =>
    {
        highContrast.Value = value;
        return SettingChangeResult.Success();
    });

agent.RegisterSetting(
    ModId,
    new SettingMetadata(
        "highContrast",
        "Accessibility",
        "HighContrast",
        new LocalizedText("High contrast"),
        SettingKind.Boolean,
        highContrast.DefaultValue,
        SettingApplyMode.Live,
        SettingDisplay.Switch),
    binding);
```

The supported wire and binding types are:

| Setting kind | Binding value |
| --- | --- |
| `Boolean` | `bool` |
| `String` | `string` |
| `Integer` | `int` or `long` |
| `Decimal` | `float`, `double`, or `decimal` |
| `SingleSelect` | `string` containing an option value |
| `MultiSelect` | `string[]` containing unique option values |

For enums or another mod-specific type, implement `ISettingBinding` and map it
to one of these wire values explicitly.

`DelegateSettingBinding<T>` raises `ValueChanged` after an accepted Agent write.
Call `NotifyValueChanged()` when the mod changes the value independently. If a
BepInEx `SettingChanged` handler is used, suppress it while applying an Agent
write so the same change is not announced twice; the complete example shows
this pattern.

## Deferred settings

`RestartRequired` and `NextLaunch` settings must persist a value without
applying it to the current game session. Supply the optional store callback:

```csharp
var binding = new DelegateSettingBinding<int>(
    () => liveValue,
    value => ApplyLive(value),
    (value, mode) => StoreForNextStart(value, mode));
```

The Agent rejects a deferred write with `deferred_write_unsupported` when no
store callback exists. Return `SettingChangeResult.Reject("stable_error_code")`
for a value the mod cannot accept. Exceptions are contained by the Agent and
reported as `binding_error`, but bindings should still handle expected failures
themselves.

## Match the catalog contract

For a catalog entry with `"integration": "agent"`:

- `modId` must equal `ModRegistration.ModId`.
- `guid` must equal the mod's `BepInPlugin` GUID.
- The catalog and registered mod versions must describe the same release.
- `compatibility.minimumAgentVersion` is required and must equal the hard
  BepInEx dependency's minimum version.
- A catalog field and a dynamic field with the same ID must have the same
  section, key, control, default, display, apply mode, range, and options.

GameTweaks blocks a dynamic field when its contract conflicts with the reviewed
catalog definition.

## Package the mod

Put the plugin DLL and only the mod's own required runtime dependencies in the
release ZIP. Never include any of these shared files:

- `GameTweaks.Agent.Abstractions.dll`
- `GameTweaks.Agent.Core.dll`
- `GameTweaks.Agent.Mono.dll`
- `GameTweaks.Agent.IL2CPP.dll`

The catalog validator and desktop installer reject mod archives containing
those names. For the example project, package
`bin/Release/netstandard2.0/Example.Accessibility.dll`, not the entire output
directory.

## Local test flow

1. Enable Developer Mode in GameTweaks.
2. Install or repair the development Agent from the game's details page while
   the game is closed.
3. Copy the plugin to `BepInEx/plugins/GameTweaks/<mod-id>/`.
4. Start the GameTweaks desktop app, then the game.
5. Confirm that the mod appears as External and that live changes work in both
   directions.

The SDK only exposes typed metadata and configuration values. It does not add
process execution, shell commands, arbitrary file access, memory access, or a
generic RPC channel.
