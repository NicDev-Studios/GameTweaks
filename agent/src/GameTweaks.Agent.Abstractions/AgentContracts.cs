using System.Diagnostics;

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

        if (typeof(T) == typeof(long) && value is int integer)
        {
            converted = (T)(object)(long)integer;
            return true;
        }
        if (typeof(T) == typeof(int) && value is long longInteger &&
            longInteger is >= int.MinValue and <= int.MaxValue)
        {
            converted = (T)(object)(int)longInteger;
            return true;
        }
        if (typeof(T) == typeof(float) && value is double floating &&
            !double.IsNaN(floating) && !double.IsInfinity(floating) &&
            floating is >= -float.MaxValue and <= float.MaxValue)
        {
            converted = (T)(object)(float)floating;
            return true;
        }
        if (typeof(T) == typeof(decimal) && value is double decimalValue &&
            !double.IsNaN(decimalValue) && !double.IsInfinity(decimalValue))
        {
            try
            {
                converted = (T)(object)(decimal)decimalValue;
                return true;
            }
            catch (OverflowException)
            {
                // The wire value cannot be represented by the requested binding type.
            }
        }

        converted = default;
        return false;
    }
}

public static class GameTweaksApi
{
    private static readonly object Gate = new();
    private static readonly Queue<AvailabilityNotification> AvailabilityNotifications = new();
    private static IGameTweaksAgent? _agent;
    private static Action<IGameTweaksAgent>? _available;
    private static bool _dispatchingAvailability;

    public static event Action<IGameTweaksAgent>? Available
    {
        add
        {
            if (value is null)
                return;

            var dispatch = false;
            lock (Gate)
            {
                _available += value;
                var agent = _agent;
                if (agent is not null)
                    AvailabilityNotifications.Enqueue(new(value, agent));
                dispatch = TryBeginAvailabilityDispatch();
            }

            if (dispatch)
                DispatchAvailability();
        }
        remove
        {
            if (value is null)
                return;
            lock (Gate)
                _available -= value;
        }
    }

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

    internal static void Attach(IGameTweaksAgent agent)
    {
        if (agent is null)
            throw new ArgumentNullException(nameof(agent));
        var dispatch = false;
        lock (Gate)
        {
            _agent = agent;
            foreach (var subscriber in _available?.GetInvocationList() ?? Array.Empty<Delegate>())
                AvailabilityNotifications.Enqueue(new(
                    (Action<IGameTweaksAgent>)subscriber,
                    agent));
            dispatch = TryBeginAvailabilityDispatch();
        }
        if (dispatch)
            DispatchAvailability();
    }

    internal static void Detach(IGameTweaksAgent agent)
    {
        if (agent is null)
            throw new ArgumentNullException(nameof(agent));
        lock (Gate)
        {
            if (ReferenceEquals(_agent, agent))
                _agent = null;
        }
    }

    private static void NotifySubscriber(
        Action<IGameTweaksAgent> subscriber,
        IGameTweaksAgent agent)
    {
        try
        {
            subscriber(agent);
        }
        catch (Exception error)
        {
            try
            {
                Trace.TraceWarning(
                    "GameTweaks Agent availability callback failed ({0}): {1}",
                    error.GetType().Name,
                    error.Message);
            }
            catch (Exception)
            {
                // A process-wide custom trace listener must not stop Agent delivery.
            }
        }
    }

    private static bool TryBeginAvailabilityDispatch()
    {
        if (_dispatchingAvailability || AvailabilityNotifications.Count == 0)
            return false;
        _dispatchingAvailability = true;
        return true;
    }

    private static void DispatchAvailability()
    {
        while (true)
        {
            AvailabilityNotification notification;
            lock (Gate)
            {
                if (AvailabilityNotifications.Count == 0)
                {
                    _dispatchingAvailability = false;
                    return;
                }
                notification = AvailabilityNotifications.Dequeue();
            }
            NotifySubscriber(notification.Subscriber, notification.Agent);
        }
    }

    private sealed record AvailabilityNotification(
        Action<IGameTweaksAgent> Subscriber,
        IGameTweaksAgent Agent);
}
