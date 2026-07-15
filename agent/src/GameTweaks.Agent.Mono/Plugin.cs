using System;
using System.IO;
using System.Security;
using System.Text.Json;
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
            var marker = AgentBootstrap.ResolveMarkerPath(Paths.PluginPath);
            _agent = AgentBootstrap.Start(marker, "mono");
            Logger.LogInfo("GameTweaks Agent loaded. Waiting for the desktop app.");
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
        Logger.LogError($"GameTweaks Agent could not start ({error.GetType().Name}): {error.Message}");

    private void OnDestroy()
    {
        _agent?.Dispose();
    }
}
