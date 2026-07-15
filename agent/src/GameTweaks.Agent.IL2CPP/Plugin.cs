using System;
using System.IO;
using BepInEx;
using BepInEx.Unity.IL2CPP;
using GameTweaks.Agent.Core;

namespace GameTweaks.Agent.IL2CPP;

[BepInPlugin("dev.gametweaks.agent", "GameTweaks Agent", "0.1.0")]
public sealed class Plugin : BasePlugin
{
    private IDisposable? _agent;

    public override void Load()
    {
        try
        {
            var marker = Path.Combine(Paths.PluginPath, "GameTweaks.Agent", ".gametweaks-agent.json");
            _agent = AgentBootstrap.Start(marker, "il2Cpp");
            Log.LogInfo("GameTweaks Agent loaded. Waiting for the desktop app.");
        }
        catch (Exception error)
        {
            Log.LogError($"GameTweaks Agent could not start: {error.Message}");
        }
    }

    public override bool Unload()
    {
        _agent?.Dispose();
        _agent = null;
        return true;
    }
}
