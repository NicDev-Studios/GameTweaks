namespace GameTweaks.Agent.Abstractions;

public enum SettingKind
{
    Boolean,
    String,
    Integer,
    Decimal,
    SingleSelect,
    MultiSelect
}

public enum SettingApplyMode
{
    Live,
    RestartRequired,
    NextLaunch
}

public enum SettingDisplay
{
    Switch,
    Checkbox,
    Text,
    Number,
    Dropdown,
    Radio,
    CheckboxGroup
}

public sealed record LocalizedText(string En, string? De = null);

public sealed record SettingOption(string Value, LocalizedText Label);

public sealed record ModRegistration(
    string ModId,
    string Version,
    LocalizedText Name,
    LocalizedText Description);

public sealed record SettingMetadata(
    string Id,
    string Section,
    string Key,
    LocalizedText Label,
    SettingKind Kind,
    object? DefaultValue,
    SettingApplyMode ApplyMode,
    SettingDisplay? Display = null,
    LocalizedText? Description = null,
    double? Minimum = null,
    double? Maximum = null,
    double? Step = null,
    int? MaximumLength = null,
    IReadOnlyList<SettingOption>? Options = null);

public sealed record SettingChangeResult(bool Accepted, string? ErrorCode = null, bool RestartRequired = false)
{
    public static SettingChangeResult Success(bool restartRequired = false) => new(true, null, restartRequired);
    public static SettingChangeResult Reject(string errorCode) => new(false, errorCode);
}

public interface ISettingBinding
{
    object? GetValue();
    SettingChangeResult SetValue(object? value);
    event Action<object?>? ValueChanged;
}

public interface IDeferredSettingBinding : ISettingBinding
{
    SettingChangeResult SetStoredValue(object? value, SettingApplyMode mode);
}

public interface IGameTweaksAgent
{
    void RegisterMod(ModRegistration mod);
    void RegisterSetting(string modId, SettingMetadata metadata, ISettingBinding binding);
    void UnregisterMod(string modId);
}

public sealed class DelegateSettingBinding<T> : IDeferredSettingBinding
{
    private readonly Func<T> _getter;
    private readonly Func<T, SettingChangeResult> _setter;
    private readonly Func<T, SettingApplyMode, SettingChangeResult>? _store;

    public DelegateSettingBinding(
        Func<T> getter,
        Func<T, SettingChangeResult> setter,
        Func<T, SettingApplyMode, SettingChangeResult>? store = null)
    {
        _getter = getter ?? throw new ArgumentNullException(nameof(getter));
        _setter = setter ?? throw new ArgumentNullException(nameof(setter));
        _store = store;
    }

    public event Action<object?>? ValueChanged;

    public object? GetValue() => _getter();

    public SettingChangeResult SetValue(object? value)
    {
        if (!TryConvert(value, out var converted))
            return SettingChangeResult.Reject("invalid_value_type");

        var result = _setter(converted!);
        if (result.Accepted)
            ValueChanged?.Invoke(converted);
        return result;
    }

    public void NotifyValueChanged() => ValueChanged?.Invoke(_getter());

    public SettingChangeResult SetStoredValue(object? value, SettingApplyMode mode)
    {
        if (mode == SettingApplyMode.Live)
            return SetValue(value);
        if (_store is null)
            return SettingChangeResult.Reject("deferred_write_unsupported");
        if (!TryConvert(value, out var converted))
            return SettingChangeResult.Reject("invalid_value_type");
        return _store(converted!, mode);
    }

    private static bool TryConvert(object? value, out T? converted)
    {
        if (value is T typed)
        {
            converted = typed;
            return true;
        }

        converted = default;
        return false;
    }
}

public static class GameTweaksApi
{
    private static readonly object Gate = new();
    private static IGameTweaksAgent? _agent;

    public static event Action<IGameTweaksAgent>? Available;

    public static bool TryGetAgent(out IGameTweaksAgent? agent)
    {
        lock (Gate)
        {
            agent = _agent;
            return agent is not null;
        }
    }

    public static IGameTweaksAgent Agent
    {
        get
        {
            if (TryGetAgent(out var agent))
                return agent!;
            throw new InvalidOperationException("The GameTweaks Agent is not loaded yet.");
        }
    }

    public static void Attach(IGameTweaksAgent agent)
    {
        if (agent is null)
            throw new ArgumentNullException(nameof(agent));
        lock (Gate)
            _agent = agent;
        Available?.Invoke(agent);
    }
}
