using System.Diagnostics;
using System.IO.Pipes;
using System.Reflection;
using System.Text.Json;
using GameTweaks.Agent.Abstractions;
using GameTweaks.Agent.Core;

namespace GameTweaks.Agent.Core.Tests;

public sealed class AgentPipeClientTests
{
    [Fact]
    public void RejectsOversizedFramesBeforeAllocation()
    {
        using var stream = new MemoryStream(BitConverter.GetBytes(1024 * 1024 + 1));
        Assert.Throws<InvalidDataException>(() => AgentPipeClient.ReadJson(stream));
    }

    [Fact]
    public void WritesLengthPrefixedProtocolFrames()
    {
        using var stream = new MemoryStream();
        AgentPipeClient.WriteJson(stream, new { type = "snapshot", protocolVersion = 1 });
        var raw = stream.ToArray();

        Assert.Equal(raw.Length - 4, BitConverter.ToInt32(raw, 0));
    }

    [Fact(Timeout = 15_000)]
    public async Task DisposeStopsAConnectedWorkerAndSnapshotWriter()
    {
        var pipeName = $"GT.{Guid.NewGuid():N}";
        using var server = new NamedPipeServerStream(
            pipeName,
            PipeDirection.InOut,
            1,
            PipeTransmissionMode.Byte,
            PipeOptions.Asynchronous);
        var client = new AgentPipeClient(
            42,
            new string('1', 64),
            "mono",
            new AgentRegistry(),
            pipeName);
        try
        {
            client.Start();
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(5));
            await server.WaitForConnectionAsync(timeout.Token);
            AgentPipeClient.WriteJson(server, new
            {
                type = "challenge",
                protocolVersion = 1,
                challenge = new string('a', 64)
            });
            using var hello = AgentPipeClient.ReadJson(server);
            Assert.Equal("hello", hello.RootElement.GetProperty("type").GetString());
            AgentPipeClient.WriteJson(server, new
            {
                type = "helloAck",
                protocolVersion = 1,
                accepted = true
            });
            using var snapshot = AgentPipeClient.ReadJson(server);
            Assert.Equal("snapshot", snapshot.RootElement.GetProperty("type").GetString());

            var stopwatch = Stopwatch.StartNew();
            client.Dispose();
            stopwatch.Stop();

            Assert.True(stopwatch.Elapsed < TimeSpan.FromMilliseconds(1500));
            client.Dispose();
        }
        finally
        {
            client.Dispose();
        }
    }

    [Fact(Timeout = 15_000)]
    public async Task BootstrapCanRestartAfterItsLifetimeIsDisposed()
    {
        var markerPath = Path.Combine(
            Path.GetTempPath(),
            $"gametweaks-agent-test-{Guid.NewGuid():N}.json");
        File.WriteAllText(markerPath, JsonSerializer.Serialize(new
        {
            schemaVersion = 1,
            appId = 42,
            runtime = "mono",
            secret = new string('1', 64)
        }));
        IDisposable? first = null;
        IDisposable? second = null;
        var beforeAttachCalls = 0;
        var afterAttachCalls = 0;
        Action<IGameTweaksAgent> throwingSubscriber = _ =>
            throw new InvalidOperationException("Test subscriber failure.");
        Action<IGameTweaksAgent> beforeAttachSubscriber = _ => beforeAttachCalls++;
        Action<IGameTweaksAgent> afterAttachSubscriber = _ => afterAttachCalls++;
        try
        {
            GameTweaksApi.Available += throwingSubscriber;
            GameTweaksApi.Available += beforeAttachSubscriber;
            first = AgentBootstrap.Start(markerPath, "mono");
            Assert.True(GameTweaksApi.TryGetAgent(out var firstRegistry));
            Assert.Equal(1, beforeAttachCalls);

            GameTweaksApi.Available += afterAttachSubscriber;
            Assert.Equal(1, afterAttachCalls);

            await Task.Run(first.Dispose);
            first.Dispose();

            Assert.False(GameTweaksApi.TryGetAgent(out _));
            second = AgentBootstrap.Start(markerPath, "mono");
            Assert.NotSame(first, second);
            Assert.True(GameTweaksApi.TryGetAgent(out var secondRegistry));
            Assert.NotSame(firstRegistry, secondRegistry);
            Assert.Equal(2, beforeAttachCalls);
            Assert.Equal(2, afterAttachCalls);

            await Task.Run(second.Dispose);
            second.Dispose();
            Assert.False(GameTweaksApi.TryGetAgent(out _));
        }
        finally
        {
            GameTweaksApi.Available -= afterAttachSubscriber;
            GameTweaksApi.Available -= beforeAttachSubscriber;
            GameTweaksApi.Available -= throwingSubscriber;
            second?.Dispose();
            first?.Dispose();
            File.Delete(markerPath);
        }
    }

    [Fact]
    public void RuntimeAttachmentIsNotPartOfThePublicSdk()
    {
        Assert.Null(typeof(GameTweaksApi).GetMethod(
            "Attach",
            BindingFlags.Public | BindingFlags.Static));
        Assert.NotNull(typeof(GameTweaksApi).GetMethod(
            "Attach",
            BindingFlags.NonPublic | BindingFlags.Static));
    }

    [Fact(Timeout = 15_000)]
    public async Task AvailabilityRemainsOrderedWhenTheAgentIsReplaced()
    {
        var first = new TestAgent();
        var second = new TestAgent();
        using var firstCallbackStarted = new ManualResetEventSlim();
        using var releaseFirstCallback = new ManualResetEventSlim();
        var received = new List<IGameTweaksAgent>();
        Action<IGameTweaksAgent> subscriber = agent =>
        {
            if (ReferenceEquals(agent, first))
            {
                firstCallbackStarted.Set();
                releaseFirstCallback.Wait(TimeSpan.FromSeconds(5));
            }
            lock (received)
                received.Add(agent);
        };

        InvokeApiMethod("Attach", first);
        var subscribe = Task.Run(() => GameTweaksApi.Available += subscriber);
        try
        {
            Assert.True(firstCallbackStarted.Wait(TimeSpan.FromSeconds(5)));
            await Task.Run(() => InvokeApiMethod("Attach", second));
            releaseFirstCallback.Set();
            await subscribe;

            lock (received)
                Assert.Equal(new IGameTweaksAgent[] { first, second }, received);
        }
        finally
        {
            releaseFirstCallback.Set();
            await subscribe;
            GameTweaksApi.Available -= subscriber;
            InvokeApiMethod("Detach", second);
            InvokeApiMethod("Detach", first);
        }
    }

    [Fact]
    public void BootstrapDoesNotPublishARegistryWhenClientConstructionFails()
    {
        var markerPath = Path.Combine(
            Path.GetTempPath(),
            $"gametweaks-agent-test-{Guid.NewGuid():N}.json");
        File.WriteAllText(markerPath, JsonSerializer.Serialize(new
        {
            schemaVersion = 1,
            appId = 42,
            runtime = "mono",
            secret = new string('z', 64)
        }));
        try
        {
            Assert.Throws<FormatException>(() => AgentBootstrap.Start(markerPath, "mono"));
            Assert.False(GameTweaksApi.TryGetAgent(out _));
        }
        finally
        {
            File.Delete(markerPath);
        }
    }

    [Fact(Timeout = 15_000)]
    public async Task SetterExceptionsReturnAStableRejectionWithoutDisconnecting()
    {
        var registry = new AgentRegistry();
        registry.RegisterMod(new("throwing.mod", "1.0.0", new("Throwing"), new("Throwing mod")));
        registry.RegisterSetting("throwing.mod", new(
            "enabled", "General", "Enabled", new("Enabled"), SettingKind.Boolean, true,
            SettingApplyMode.Live), new ThrowingSetterBinding());
        var pipeName = $"GT.{Guid.NewGuid():N}";
        using var server = CreatePipeServer(pipeName);
        using var client = new AgentPipeClient(
            42,
            new string('1', 64),
            "mono",
            registry,
            pipeName);
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(5));

        client.Start();
        await AcceptAgentAsync(server, timeout.Token);
        using var snapshot = AgentPipeClient.ReadJson(server);
        Assert.Equal("snapshot", snapshot.RootElement.GetProperty("type").GetString());

        for (var request = 0; request < 2; request++)
        {
            AgentPipeClient.WriteJson(server, new
            {
                type = "setConfig",
                protocolVersion = 1,
                requestId = $"request-{request}",
                modId = "throwing.mod",
                values = new { enabled = false }
            });
            using var result = AgentPipeClient.ReadJson(server);
            Assert.Equal("configResult", result.RootElement.GetProperty("type").GetString());
            Assert.False(result.RootElement.GetProperty("accepted").GetBoolean());
            Assert.Equal("binding_error", result.RootElement.GetProperty("errorCode").GetString());
        }
    }

    [Fact(Timeout = 15_000)]
    public async Task GetterExceptionsDisconnectThePipeAndAllowReconnect()
    {
        var registry = new AgentRegistry();
        registry.RegisterMod(new("throwing.mod", "1.0.0", new("Throwing"), new("Throwing mod")));
        registry.RegisterSetting("throwing.mod", new(
            "enabled", "General", "Enabled", new("Enabled"), SettingKind.Boolean, true,
            SettingApplyMode.Live), new ThrowingGetterBinding());
        var pipeName = $"GT.{Guid.NewGuid():N}";
        using var client = new AgentPipeClient(
            42,
            new string('1', 64),
            "mono",
            registry,
            pipeName);
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(8));
        client.Start();

        using (var firstServer = CreatePipeServer(pipeName))
        {
            await AcceptAgentAsync(firstServer, timeout.Token);
            var read = Task.Run(() => AgentPipeClient.ReadJson(firstServer));
            await Assert.ThrowsAnyAsync<IOException>(async () =>
            {
                using var frame = await read.WaitAsync(timeout.Token);
            });
        }

        using var secondServer = CreatePipeServer(pipeName);
        await AcceptAgentAsync(secondServer, timeout.Token);
    }

    private static NamedPipeServerStream CreatePipeServer(string pipeName) => new(
        pipeName,
        PipeDirection.InOut,
        1,
        PipeTransmissionMode.Byte,
        PipeOptions.Asynchronous);

    private static async Task AcceptAgentAsync(
        NamedPipeServerStream server,
        CancellationToken cancellationToken)
    {
        await server.WaitForConnectionAsync(cancellationToken);
        AgentPipeClient.WriteJson(server, new
        {
            type = "challenge",
            protocolVersion = 1,
            challenge = new string('a', 64)
        });
        using var hello = AgentPipeClient.ReadJson(server);
        Assert.Equal("hello", hello.RootElement.GetProperty("type").GetString());
        AgentPipeClient.WriteJson(server, new
        {
            type = "helloAck",
            protocolVersion = 1,
            accepted = true
        });
    }

    private static void InvokeApiMethod(string name, IGameTweaksAgent agent)
    {
        var method = typeof(GameTweaksApi).GetMethod(
            name,
            BindingFlags.NonPublic | BindingFlags.Static) ??
            throw new InvalidOperationException($"GameTweaksApi.{name} was missing.");
        method.Invoke(null, new object[] { agent });
    }

    private sealed class TestAgent : IGameTweaksAgent
    {
        public void RegisterMod(ModRegistration mod)
        {
        }

        public void RegisterSetting(
            string modId,
            SettingMetadata metadata,
            ISettingBinding binding)
        {
        }

        public void UnregisterMod(string modId)
        {
        }
    }

    private sealed class ThrowingSetterBinding : ISettingBinding
    {
        public event Action<object?>? ValueChanged
        {
            add { }
            remove { }
        }

        public object? GetValue() => true;

        public SettingChangeResult SetValue(object? value) =>
            throw new InvalidOperationException("Test setter failure.");
    }

    private sealed class ThrowingGetterBinding : ISettingBinding
    {
        public event Action<object?>? ValueChanged
        {
            add { }
            remove { }
        }

        public object? GetValue() => throw new InvalidOperationException("Test getter failure.");

        public SettingChangeResult SetValue(object? value) => SettingChangeResult.Success();
    }
}
