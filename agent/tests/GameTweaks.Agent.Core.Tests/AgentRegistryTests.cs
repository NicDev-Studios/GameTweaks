using GameTweaks.Agent.Abstractions;
using GameTweaks.Agent.Core;

namespace GameTweaks.Agent.Core.Tests;

public sealed class AgentRegistryTests
{
    [Fact]
    public void RegistersMultipleModsAndTypedSettings()
    {
        var registry = new AgentRegistry();
        registry.RegisterMod(new("first.mod", "1.0.0", new("First"), new("First mod")));
        registry.RegisterMod(new("second.mod", "2.0.0", new("Second"), new("Second mod")));
        registry.RegisterSetting("first.mod", new(
            "enabled", "General", "Enabled", new("Enabled"), SettingKind.Boolean, true, SettingApplyMode.Live),
            new TestBinding(true));

        Assert.Equal(2, registry.Snapshot().Count);
        Assert.True(registry.SetValue("first.mod", "enabled", false).Accepted);
    }

    [Fact]
    public void RejectsUnsafeIdentifiers()
    {
        var registry = new AgentRegistry();
        Assert.Throws<ArgumentException>(() => registry.RegisterMod(
            new("../unsafe", "1.0.0", new("Unsafe"), new("Unsafe mod"))));
    }

    [Fact]
    public void RejectsInvalidTypedDefaults()
    {
        var registry = new AgentRegistry();
        registry.RegisterMod(new("typed.mod", "1.0.0", new("Typed"), new("Typed mod")));

        Assert.Throws<ArgumentException>(() => registry.RegisterSetting("typed.mod", new(
            "speed", "General", "Speed", new("Speed"), SettingKind.Integer, "fast",
            SettingApplyMode.Live, Minimum: 0, Maximum: 10, Step: 1), new TestBinding(1)));
    }

    [Fact]
    public void PropagatesStableSettingRejections()
    {
        var registry = new AgentRegistry();
        registry.RegisterMod(new("reject.mod", "1.0.0", new("Reject"), new("Reject mod")));
        registry.RegisterSetting("reject.mod", new(
            "enabled", "General", "Enabled", new("Enabled"), SettingKind.Boolean, true,
            SettingApplyMode.Live), new RejectingBinding());

        var result = registry.SetValue("reject.mod", "enabled", false);

        Assert.False(result.Accepted);
        Assert.Equal("mod_rejected", result.ErrorCode);
    }

    [Fact]
    public void DeferredBindingsStoreWithoutApplyingToTheLiveSession()
    {
        var liveValue = true;
        bool? storedValue = null;
        var binding = new DelegateSettingBinding<bool>(
            () => liveValue,
            value =>
            {
                liveValue = value;
                return SettingChangeResult.Success();
            },
            (value, mode) =>
            {
                storedValue = value;
                return SettingChangeResult.Success(mode == SettingApplyMode.RestartRequired);
            });

        var result = binding.SetStoredValue(false, SettingApplyMode.RestartRequired);

        Assert.True(result.Accepted);
        Assert.True(result.RestartRequired);
        Assert.True(liveValue);
        Assert.False(storedValue);
    }

    [Fact]
    public void DeferredBindingsRejectChangesWithoutPersistence()
    {
        var binding = new DelegateSettingBinding<bool>(
            () => true,
            _ => SettingChangeResult.Success());

        var result = binding.SetStoredValue(false, SettingApplyMode.NextLaunch);

        Assert.False(result.Accepted);
        Assert.Equal("deferred_write_unsupported", result.ErrorCode);
    }

    [Fact]
    public void DelegateBindingsConvertValidatedWireNumbers()
    {
        long integer = 0;
        float floating = 0;
        decimal decimalValue = 0;
        var integerBinding = new DelegateSettingBinding<long>(
            () => integer,
            value =>
            {
                integer = value;
                return SettingChangeResult.Success();
            });
        var floatingBinding = new DelegateSettingBinding<float>(
            () => floating,
            value =>
            {
                floating = value;
                return SettingChangeResult.Success();
            });
        var decimalBinding = new DelegateSettingBinding<decimal>(
            () => decimalValue,
            value =>
            {
                decimalValue = value;
                return SettingChangeResult.Success();
            });

        Assert.True(integerBinding.SetValue(42).Accepted);
        Assert.True(floatingBinding.SetValue(0.5d).Accepted);
        Assert.True(decimalBinding.SetValue(1.25d).Accepted);
        Assert.Equal(42, integer);
        Assert.Equal(0.5f, floating);
        Assert.Equal(1.25m, decimalValue);
    }

    private sealed class TestBinding(object? value) : ISettingBinding
    {
        private object? _value = value;
        public event Action<object?>? ValueChanged;
        public object? GetValue() => _value;
        public SettingChangeResult SetValue(object? value)
        {
            _value = value;
            ValueChanged?.Invoke(value);
            return SettingChangeResult.Success();
        }
    }

    private sealed class RejectingBinding : ISettingBinding
    {
        public event Action<object?>? ValueChanged
        {
            add { }
            remove { }
        }
        public object? GetValue() => true;
        public SettingChangeResult SetValue(object? value) => SettingChangeResult.Reject("mod_rejected");
    }
}
