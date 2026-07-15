using System;
using System.IO;
using BepInEx;
using GameTweaks.Agent.Core;

namespace GameTweaks.Agent.Mono;

[BepInPlugin("dev.gametweaks.agent", "GameTweaks Agent", "0.1.0")]
public sealed class Plugin : BaseUnityPlugin
{
    private IDisposable? _agent;

    private void Awake()
    {
        try
        {
            var marker = Path.Combine(Paths.PluginPath, "GameTweaks.Agent", ".gametweaks-agent.json");
            _agent = AgentBootstrap.Start(marker, "mono");
            Logger.LogInfo("GameTweaks Agent loaded. Waiting for the desktop app.");
        }
        catch (Exception error)
        {
            Logger.LogError($"GameTweaks Agent could not start: {error.Message}");
        }
    }

    private void OnDestroy()
    {
        _agent?.Dispose();
    }
}
