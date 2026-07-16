# Mono SDK example

This is a minimal BepInEx 5 Mono plugin that exposes one existing
`ConfigEntry<bool>` through the shared GameTweaks Agent.

After `GameTweaks.Agent.Abstractions` version `0.1.0` is published, build it
with:

```sh
dotnet build --configuration Release
```

The repository's `agent/pack-sdk.ps1` script instead restores the example from
the freshly packed local NuGet file, so CI does not depend on a package that has
already been published.

`UnityEngine.Reference` only supplies the compile-time `MonoBehaviour` identity
needed by BepInEx. It is not copied to the output. A real mod that uses Unity or
game APIs must reference the assemblies from the exact game version it targets.

Package only
`bin/Release/netstandard2.0/Example.Accessibility.dll` in the mod ZIP. BepInEx,
Unity, and all `GameTweaks.Agent.*` assemblies are runtime-owned dependencies
and must not be included.
