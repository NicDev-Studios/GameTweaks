using System;
using System.IO;
using System.Security;
using System.Text.Json;
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
            var marker = AgentBootstrap.ResolveMarkerPath(Paths.PluginPath);
            _agent = AgentBootstrap.Start(marker, "il2Cpp");
            Log.LogInfo("GameTweaks Agent loaded. Waiting for the desktop app.");
        }
        catch (IOException error)
        {
            LogStartupError(error);
        }
        catch (UnauthorizedAccessException error)
        {
            LogStartupError(error);
        }
        catch (JsonException error)
        {
            LogStartupError(error);
        }
        catch (ArgumentException error)
        {
            LogStartupError(error);
        }
        catch (FormatException error)
        {
            LogStartupError(error);
        }
        catch (SecurityException error)
        {
            LogStartupError(error);
        }
    }

    private void LogStartupError(Exception error) =>
        Log.LogError($"GameTweaks Agent could not start ({error.GetType().Name}): {error.Message}");

    public override bool Unload()
    {
        _agent?.Dispose();
        _agent = null;
        return true;
    }
}
