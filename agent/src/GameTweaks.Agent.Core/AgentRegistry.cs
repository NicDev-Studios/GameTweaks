using System.Collections.Concurrent;
using GameTweaks.Agent.Abstractions;

namespace GameTweaks.Agent.Core;

public sealed class AgentRegistry : IGameTweaksAgent
{
    private const int MaximumMods = 256;
    private const int MaximumSettingsPerMod = 512;
    private readonly ConcurrentDictionary<string, RegisteredMod> _mods = new(StringComparer.Ordinal);

    public event Action? Changed;

    public void RegisterMod(ModRegistration mod)
    {
        ValidateId(mod.ModId, nameof(mod.ModId));
        ValidateText(mod.Version, 64, nameof(mod.Version));
        ValidateText(mod.Name.En, 512, nameof(mod.Name));
        ValidateText(mod.Description.En, 512, nameof(mod.Description));
        if (_mods.Count >= MaximumMods && !_mods.ContainsKey(mod.ModId))
            throw new InvalidOperationException("The Agent mod limit was reached.");
        if (!_mods.TryAdd(mod.ModId, new RegisteredMod(mod)))
            throw new InvalidOperationException("The mod ID is already registered.");
        Changed?.Invoke();
    }

    public void RegisterSetting(string modId, SettingMetadata metadata, ISettingBinding binding)
    {
        if (binding is null)
            throw new ArgumentNullException(nameof(binding));
        if (!_mods.TryGetValue(modId, out var mod))
            throw new InvalidOperationException("Register the mod before its settings.");
        ValidateSetting(metadata);
        if (mod.Settings.Count >= MaximumSettingsPerMod)
            throw new InvalidOperationException("The per-mod setting limit was reached.");
        if (!mod.Settings.TryAdd(metadata.Id, new RegisteredSetting(metadata, binding)))
            throw new InvalidOperationException("The setting ID is already registered.");
        binding.ValueChanged += _ => Changed?.Invoke();
        Changed?.Invoke();
    }

    public void UnregisterMod(string modId)
    {
        _mods.TryRemove(modId, out _);
        Changed?.Invoke();
    }

    public IReadOnlyCollection<RegisteredMod> Snapshot() => _mods.Values.ToArray();

    public SettingChangeResult SetValue(string modId, string settingId, object? value)
    {
        if (!_mods.TryGetValue(modId, out var mod) || !mod.Settings.TryGetValue(settingId, out var setting))
            return SettingChangeResult.Reject("unknown_setting");
        return setting.Binding.SetValue(value);
    }

    internal bool TryGetSetting(string modId, string settingId, out RegisteredSetting? setting)
    {
        setting = null;
        return _mods.TryGetValue(modId, out var mod) &&
               mod.Settings.TryGetValue(settingId, out setting);
    }

    private static void ValidateSetting(SettingMetadata metadata)
    {
        ValidateId(metadata.Id, nameof(metadata.Id));
        ValidateConfigName(metadata.Section, nameof(metadata.Section));
        ValidateConfigName(metadata.Key, nameof(metadata.Key));
        ValidateText(metadata.Label.En, 512, nameof(metadata.Label));
        if (metadata.Kind is SettingKind.Integer or SettingKind.Decimal)
        {
            if (metadata.Minimum is null || metadata.Maximum is null || metadata.Step is null ||
                metadata.Minimum > metadata.Maximum || metadata.Step <= 0)
                throw new ArgumentException("Numeric settings need a valid range and step.");
        }
        if (metadata.Kind == SettingKind.String && metadata.MaximumLength is not > 0 and <= 65535)
            throw new ArgumentException("String settings need a maximum length.");
        if (metadata.Kind is SettingKind.SingleSelect or SettingKind.MultiSelect &&
            (metadata.Options is null || metadata.Options.Count == 0 || metadata.Options.Count > 256))
            throw new ArgumentException("Selection settings need bounded options.");
        if (metadata.Options is not null && metadata.Options.Any(option =>
                string.IsNullOrEmpty(option.Value) || option.Value.Length > 128 ||
                string.IsNullOrWhiteSpace(option.Label.En) || option.Label.En.Length > 512))
            throw new ArgumentException("A selection option is invalid.");
        if (metadata.Options is not null && metadata.Options.Select(option => option.Value)
                .Distinct(StringComparer.Ordinal).Count() != metadata.Options.Count)
            throw new ArgumentException("Selection option values must be unique.");
        if (!DefaultMatchesKind(metadata))
            throw new ArgumentException("The setting default does not match its declared type.");
    }

    private static void ValidateId(string value, string name)
    {
        ValidateText(value, 96, name);
        if (value.Any(character => !IsAsciiLetterOrDigit(character) && character is not '.' and not '_' and not '-'))
            throw new ArgumentException("IDs may only use ASCII letters, digits, dots, underscores and dashes.", name);
    }

    private static void ValidateConfigName(string value, string name)
    {
        ValidateText(value, 128, name);
        if (value.IndexOfAny(new[] { '\r', '\n', '[', ']', '=' }) >= 0)
            throw new ArgumentException("The config section or key contains unsafe characters.", name);
    }

    private static void ValidateText(string value, int maximum, string name)
    {
        if (string.IsNullOrWhiteSpace(value) || value.Length > maximum || value.IndexOf('\0') >= 0)
            throw new ArgumentException("The value is empty or exceeds its safe limit.", name);
    }

    private static bool DefaultMatchesKind(SettingMetadata metadata) => metadata.Kind switch
    {
        SettingKind.Boolean => metadata.DefaultValue is bool,
        SettingKind.String => metadata.DefaultValue is string text &&
                              metadata.MaximumLength is int maximumLength &&
                              text.Length <= maximumLength,
        SettingKind.Integer => TryNumber(metadata.DefaultValue, out var integer) &&
                               Math.Truncate(integer) == integer && InRange(metadata, integer),
        SettingKind.Decimal => TryNumber(metadata.DefaultValue, out var number) &&
                               !double.IsNaN(number) && !double.IsInfinity(number) &&
                               InRange(metadata, number),
        SettingKind.SingleSelect => metadata.DefaultValue is string selected &&
                                    metadata.Options!.Any(option => option.Value == selected),
        SettingKind.MultiSelect => metadata.DefaultValue is IEnumerable<string> selected &&
                                   selected.Distinct(StringComparer.Ordinal).Count() == selected.Count() &&
                                   selected.All(value => metadata.Options!.Any(option => option.Value == value)),
        _ => false
    };

    private static bool TryNumber(object? value, out double number)
    {
        switch (value)
        {
            case int integer:
                number = integer;
                return true;
            case long integer:
                number = integer;
                return true;
            case float floating:
                number = floating;
                return true;
            case double floating:
                number = floating;
                return true;
            case decimal decimalValue:
                number = (double)decimalValue;
                return true;
            default:
                number = 0;
                return false;
        }
    }

    private static bool InRange(SettingMetadata metadata, double value) =>
        metadata.Minimum is double minimum && metadata.Maximum is double maximum &&
        metadata.Step is double step && step > 0 && value >= minimum && value <= maximum &&
        Math.Abs(((value - minimum) / step) - Math.Round((value - minimum) / step)) <= 0.000000001;

    private static bool IsAsciiLetterOrDigit(char value) =>
        value is >= 'a' and <= 'z' or >= 'A' and <= 'Z' or >= '0' and <= '9';
}

public sealed record RegisteredSetting(SettingMetadata Metadata, ISettingBinding Binding);

public sealed class RegisteredMod
{
    internal RegisteredMod(ModRegistration metadata) => Metadata = metadata;
    public ModRegistration Metadata { get; }
    public ConcurrentDictionary<string, RegisteredSetting> Settings { get; } = new(StringComparer.Ordinal);
}
