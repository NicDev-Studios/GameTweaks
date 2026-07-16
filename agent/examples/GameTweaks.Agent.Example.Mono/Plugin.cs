using System;
using System.Threading;
using BepInEx;
using BepInEx.Configuration;
using GameTweaks.Agent.Abstractions;

namespace Example.Accessibility;

internal static class PluginMetadata
{
    internal const string Guid = "dev.example.accessibility";
    internal const string Name = "Accessibility Example";
    internal const string Version = "1.0.0";
    internal const string AgentGuid = "dev.gametweaks.agent";
    internal const string MinimumAgentVersion = "0.1.0";
}

[BepInPlugin(PluginMetadata.Guid, PluginMetadata.Name, PluginMetadata.Version)]
[BepInDependency(PluginMetadata.AgentGuid, PluginMetadata.MinimumAgentVersion)]
public sealed class Plugin : BaseUnityPlugin
{
    private const string ModId = "example.accessibility";

    private ConfigEntry<bool>? _highContrast;
    private DelegateSettingBinding<bool>? _highContrastBinding;
    private IGameTweaksAgent? _agent;
    private int _applyingFromAgent;

    private void Awake()
    {
        var highContrast = Config.Bind(
            "Accessibility",
            "HighContrast",
            false,
            "Enables high-contrast rendering.");
        highContrast.SettingChanged += OnHighContrastChanged;
        _highContrast = highContrast;
        GameTweaksApi.Available += RegisterWithAgent;
    }

    private void RegisterWithAgent(IGameTweaksAgent agent)
    {
        if (ReferenceEquals(_agent, agent))
            return;

        UnregisterCurrentAgent();
        var entry = _highContrast;
        if (entry is null)
            return;

        var binding = new DelegateSettingBinding<bool>(
            () => entry.Value,
            value => SetHighContrast(entry, value));
        var modRegistered = false;
        try
        {
            agent.RegisterMod(new ModRegistration(
                ModId,
                PluginMetadata.Version,
                new LocalizedText(PluginMetadata.Name),
                new LocalizedText("Accessibility controls")));
            modRegistered = true;

            agent.RegisterSetting(
                ModId,
                new SettingMetadata(
                    "highContrast",
                    "Accessibility",
                    "HighContrast",
                    new LocalizedText("High contrast"),
                    SettingKind.Boolean,
                    entry.DefaultValue,
                    SettingApplyMode.Live,
                    SettingDisplay.Switch),
                binding);

            _highContrastBinding = binding;
            _agent = agent;
        }
        catch (Exception error)
        {
            if (modRegistered)
                TryUnregister(agent);
            Logger.LogError($"GameTweaks registration failed: {error}");
        }
    }

    private SettingChangeResult SetHighContrast(ConfigEntry<bool> entry, bool value)
    {
        Interlocked.Increment(ref _applyingFromAgent);
        try
        {
            entry.Value = value;
            return SettingChangeResult.Success();
        }
        catch (Exception error)
        {
            Logger.LogError($"Could not update HighContrast: {error}");
            return SettingChangeResult.Reject("config_write_failed");
        }
        finally
        {
            Interlocked.Decrement(ref _applyingFromAgent);
        }
    }

    private void OnHighContrastChanged(object? sender, EventArgs eventArgs)
    {
        if (Volatile.Read(ref _applyingFromAgent) == 0)
            _highContrastBinding?.NotifyValueChanged();
    }

    private void OnDestroy()
    {
        GameTweaksApi.Available -= RegisterWithAgent;
        if (_highContrast is not null)
            _highContrast.SettingChanged -= OnHighContrastChanged;
        UnregisterCurrentAgent();
        _highContrast = null;
    }

    private void UnregisterCurrentAgent()
    {
        var agent = _agent;
        _agent = null;
        _highContrastBinding = null;
        if (agent is not null)
            TryUnregister(agent);
    }

    private void TryUnregister(IGameTweaksAgent agent)
    {
        try
        {
            agent.UnregisterMod(ModId);
        }
        catch (Exception error)
        {
            Logger.LogError($"GameTweaks unregistration failed: {error}");
        }
    }
}
