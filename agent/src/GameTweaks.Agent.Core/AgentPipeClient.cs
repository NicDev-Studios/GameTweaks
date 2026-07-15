using System.Diagnostics;
using System.IO.Pipes;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using GameTweaks.Agent.Abstractions;

namespace GameTweaks.Agent.Core;

public static class AgentBootstrap
{
    private static readonly object Gate = new();
    private static AgentPipeClient? _client;

    public static IDisposable Start(string markerPath, string runtime)
    {
        lock (Gate)
        {
            if (_client is not null)
                return _client;

            var marker = AgentInstallMarker.Read(markerPath);
            if (!string.Equals(marker.Runtime, runtime, StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("The GameTweaks Agent marker runtime is incompatible.");

            var registry = new AgentRegistry();
            GameTweaksApi.Attach(registry);
            _client = new AgentPipeClient(marker.AppId, marker.Secret, runtime, registry);
            _client.Start();
            return _client;
        }
    }
}

public sealed class AgentPipeClient : IDisposable
{
    private const int ProtocolVersion = 1;
    private const int MaximumFrameBytes = 1024 * 1024;
    private const string AgentVersion = "0.1.0";
    private readonly uint _appId;
    private readonly byte[] _secret;
    private readonly string _runtime;
    private readonly AgentRegistry _registry;
    private readonly string _instanceId = Guid.NewGuid().ToString("N");
    private readonly ManualResetEvent _shutdown = new(false);
    private readonly AutoResetEvent _snapshotChanged = new(true);
    private readonly object _writeGate = new();
    private Thread? _worker;

    public AgentPipeClient(uint appId, string secretHex, string runtime, AgentRegistry registry)
    {
        _appId = appId;
        _secret = DecodeHex(secretHex);
        _runtime = runtime;
        _registry = registry ?? throw new ArgumentNullException(nameof(registry));
        if (_secret.Length != 32)
            throw new ArgumentException("The Agent secret must be 256-bit.", nameof(secretHex));
        _registry.Changed += HandleRegistryChanged;
    }

    public void Start()
    {
        if (_worker is not null)
            return;
        _worker = new Thread(Run) { IsBackground = true, Name = "GameTweaks Agent" };
        _worker.Start();
    }

    private void Run()
    {
        var delayMilliseconds = 1000;
        while (!_shutdown.WaitOne(0))
        {
            try
            {
                using (var pipe = new NamedPipeClientStream(
                           ".", "GameTweaks.Agent.v1", PipeDirection.InOut, PipeOptions.None))
                {
                    pipe.Connect(5000);
                    Authenticate(pipe);
                    delayMilliseconds = 1000;
                    RunConnected(pipe);
                }
            }
            catch when (!_shutdown.WaitOne(0))
            {
                if (_shutdown.WaitOne(delayMilliseconds))
                    return;
                delayMilliseconds = Math.Min(delayMilliseconds * 2, 30000);
            }
        }
    }

    private void Authenticate(Stream pipe)
    {
        using var challengeFrame = ReadJson(pipe);
        var root = challengeFrame.RootElement;
        if (root.GetProperty("type").GetString() != "challenge" ||
            root.GetProperty("protocolVersion").GetInt32() != ProtocolVersion)
            throw new InvalidDataException("The GameTweaks Agent challenge was invalid.");

        var challenge = root.GetProperty("challenge").GetString() ?? throw new InvalidDataException();
        var processId = Process.GetCurrentProcess().Id;
        var proofInput = $"{challenge}|{_appId}|{processId}|{_instanceId}";
        string proof;
        using (var hmac = new HMACSHA256(_secret))
            proof = EncodeHex(hmac.ComputeHash(Encoding.UTF8.GetBytes(proofInput)));

        WriteJson(pipe, new
        {
            type = "hello",
            protocolVersion = ProtocolVersion,
            appId = _appId,
            processId,
            instanceId = _instanceId,
            runtime = _runtime,
            agentVersion = AgentVersion,
            proof
        });

        using var acknowledgement = ReadJson(pipe);
        if (!acknowledgement.RootElement.TryGetProperty("accepted", out var accepted) ||
            !accepted.GetBoolean())
            throw new InvalidDataException("Agent authentication was rejected.");
    }

    private void RunConnected(Stream pipe)
    {
        _snapshotChanged.Set();
        var writer = new Thread(() => WriteSnapshots(pipe))
        {
            IsBackground = true,
            Name = "GameTweaks Agent snapshots"
        };
        writer.Start();
        try
        {
            while (!_shutdown.WaitOne(0))
            {
                using var message = ReadJson(pipe);
                HandleMessage(pipe, message.RootElement);
            }
        }
        finally
        {
            _snapshotChanged.Set();
        }
    }

    private void WriteSnapshots(Stream pipe)
    {
        while (!_shutdown.WaitOne(0))
        {
            var signaled = WaitHandle.WaitAny(new WaitHandle[] { _shutdown, _snapshotChanged });
            if (signaled == 0)
                return;
            try
            {
                var mods = CreateSnapshot();
                WriteJsonLocked(pipe, new
                {
                    type = "snapshot",
                    protocolVersion = ProtocolVersion,
                    appId = _appId,
                    instanceId = _instanceId,
                    mods
                });
            }
            catch
            {
                return;
            }
        }
    }

    private void HandleMessage(Stream pipe, JsonElement message)
    {
        if (message.GetProperty("type").GetString() != "setConfig" ||
            message.GetProperty("protocolVersion").GetInt32() != ProtocolVersion)
            throw new InvalidDataException("Unsupported Agent message.");

        var requestId = message.GetProperty("requestId").GetString() ?? throw new InvalidDataException();
        var modId = message.GetProperty("modId").GetString() ?? throw new InvalidDataException();
        var values = message.GetProperty("values");
        if (values.ValueKind != JsonValueKind.Object || values.EnumerateObject().Count() > 128)
            throw new InvalidDataException("The Agent setting payload was invalid.");

        string? errorCode = null;
        var restartRequired = false;
        foreach (var property in values.EnumerateObject())
        {
            if (!_registry.TryGetSetting(modId, property.Name, out var setting) || setting is null)
            {
                errorCode = "unknown_setting";
                break;
            }

            if (!TryConvertValue(setting.Metadata, property.Value, out var converted))
            {
                errorCode = "invalid_value";
                break;
            }

            var result = setting.Metadata.ApplyMode == SettingApplyMode.Live
                ? setting.Binding.SetValue(converted)
                : setting.Binding is IDeferredSettingBinding deferred
                    ? deferred.SetStoredValue(converted, setting.Metadata.ApplyMode)
                    : SettingChangeResult.Reject("deferred_write_unsupported");
            if (!result.Accepted)
            {
                errorCode = result.ErrorCode ?? "rejected";
                break;
            }
            restartRequired |= result.RestartRequired ||
                               setting.Metadata.ApplyMode == SettingApplyMode.RestartRequired;
        }

        WriteJsonLocked(pipe, new
        {
            type = "configResult",
            protocolVersion = ProtocolVersion,
            requestId,
            accepted = errorCode is null,
            errorCode,
            restartRequired
        });
    }

    private object[] CreateSnapshot() => _registry.Snapshot()
        .OrderBy(mod => mod.Metadata.ModId, StringComparer.Ordinal)
        .Select(mod => new
        {
            modId = mod.Metadata.ModId,
            version = mod.Metadata.Version,
            name = mod.Metadata.Name,
            description = mod.Metadata.Description,
            fields = mod.Settings.Values.OrderBy(setting => setting.Metadata.Id, StringComparer.Ordinal)
                .Select(setting => CreateField(setting.Metadata)).ToArray(),
            values = mod.Settings.Values.ToDictionary(
                setting => setting.Metadata.Id,
                setting => setting.Binding.GetValue(),
                StringComparer.Ordinal)
        })
        .Cast<object>()
        .ToArray();

    private static Dictionary<string, object?> CreateField(SettingMetadata metadata)
    {
        var field = new Dictionary<string, object?>(StringComparer.Ordinal)
        {
            ["control"] = ControlName(metadata.Kind),
            ["id"] = metadata.Id,
            ["section"] = metadata.Section,
            ["key"] = metadata.Key,
            ["label"] = metadata.Label,
            ["description"] = metadata.Description,
            ["default"] = metadata.DefaultValue,
            ["applyMode"] = ApplyModeName(metadata.ApplyMode)
        };
        if (metadata.Kind == SettingKind.Boolean)
            field["display"] = metadata.Display == SettingDisplay.Checkbox ? "checkbox" : "switch";
        if (metadata.Kind == SettingKind.String)
            field["maxLength"] = metadata.MaximumLength;
        if (metadata.Kind is SettingKind.Integer or SettingKind.Decimal)
        {
            field["min"] = metadata.Minimum;
            field["max"] = metadata.Maximum;
            field["step"] = metadata.Step;
        }
        if (metadata.Kind is SettingKind.SingleSelect or SettingKind.MultiSelect)
            field["options"] = metadata.Options;
        if (metadata.Kind == SettingKind.SingleSelect)
            field["display"] = metadata.Display == SettingDisplay.Radio ? "radio" : "dropdown";
        return field;
    }

    private static bool TryConvertValue(SettingMetadata metadata, JsonElement value, out object? converted)
    {
        converted = null;
        switch (metadata.Kind)
        {
            case SettingKind.Boolean when value.ValueKind is JsonValueKind.True or JsonValueKind.False:
                converted = value.GetBoolean();
                return true;
            case SettingKind.String when value.ValueKind == JsonValueKind.String:
            case SettingKind.SingleSelect when value.ValueKind == JsonValueKind.String:
                var text = value.GetString() ?? string.Empty;
                if (metadata.MaximumLength is int maximumLength && text.Length > maximumLength)
                    return false;
                if (metadata.Options is not null && !metadata.Options.Any(option => option.Value == text))
                    return false;
                converted = text;
                return true;
            case SettingKind.Integer when value.ValueKind == JsonValueKind.Number && value.TryGetInt64(out var integer):
                if (!InRange(metadata, integer) || integer % 1 != 0)
                    return false;
                converted = integer is >= int.MinValue and <= int.MaxValue ? (object)(int)integer : integer;
                return true;
            case SettingKind.Decimal when value.ValueKind == JsonValueKind.Number && value.TryGetDouble(out var number):
                if (double.IsNaN(number) || double.IsInfinity(number) || !InRange(metadata, number))
                    return false;
                converted = number;
                return true;
            case SettingKind.MultiSelect when value.ValueKind == JsonValueKind.Array:
                var selected = new List<string>();
                foreach (var item in value.EnumerateArray())
                {
                    if (item.ValueKind != JsonValueKind.String)
                        return false;
                    var selectedValue = item.GetString() ?? string.Empty;
                    if (selected.Contains(selectedValue, StringComparer.Ordinal) ||
                        metadata.Options is null ||
                        !metadata.Options.Any(option => option.Value == selectedValue))
                        return false;
                    selected.Add(selectedValue);
                }
                converted = selected.ToArray();
                return true;
            default:
                return false;
        }
    }

    private static bool InRange(SettingMetadata metadata, double value) =>
        metadata.Minimum is double minimum && metadata.Maximum is double maximum &&
        metadata.Step is double step && step > 0 && value >= minimum && value <= maximum &&
        Math.Abs(((value - minimum) / step) - Math.Round((value - minimum) / step)) <= 0.000000001;

    private static string ControlName(SettingKind kind) => kind switch
    {
        SettingKind.Boolean => "boolean",
        SettingKind.String => "string",
        SettingKind.Integer => "integer",
        SettingKind.Decimal => "decimal",
        SettingKind.SingleSelect => "singleSelect",
        SettingKind.MultiSelect => "multiSelect",
        _ => throw new ArgumentOutOfRangeException(nameof(kind))
    };

    private static string ApplyModeName(SettingApplyMode mode) => mode switch
    {
        SettingApplyMode.Live => "live",
        SettingApplyMode.RestartRequired => "restartRequired",
        SettingApplyMode.NextLaunch => "nextLaunch",
        _ => throw new ArgumentOutOfRangeException(nameof(mode))
    };

    private void HandleRegistryChanged() => _snapshotChanged.Set();

    private void WriteJsonLocked(Stream stream, object message)
    {
        lock (_writeGate)
            WriteJson(stream, message);
    }

    internal static JsonDocument ReadJson(Stream stream)
    {
        var lengthBytes = new byte[4];
        ReadExactly(stream, lengthBytes);
        var length = BitConverter.ToInt32(lengthBytes, 0);
        if (length <= 0 || length > MaximumFrameBytes)
            throw new InvalidDataException("Invalid Agent frame length.");
        var body = new byte[length];
        ReadExactly(stream, body);
        return JsonDocument.Parse(body);
    }

    internal static void WriteJson(Stream stream, object message)
    {
        var body = JsonSerializer.SerializeToUtf8Bytes(message, JsonOptions);
        if (body.Length > MaximumFrameBytes)
            throw new InvalidDataException("Agent frame exceeds the limit.");
        var length = BitConverter.GetBytes(body.Length);
        stream.Write(length, 0, length.Length);
        stream.Write(body, 0, body.Length);
        stream.Flush();
    }

    private static void ReadExactly(Stream stream, byte[] buffer)
    {
        var offset = 0;
        while (offset < buffer.Length)
        {
            var read = stream.Read(buffer, offset, buffer.Length - offset);
            if (read == 0)
                throw new EndOfStreamException();
            offset += read;
        }
    }

    private static byte[] DecodeHex(string value)
    {
        if (value is null || value.Length % 2 != 0)
            throw new ArgumentException("The Agent secret was invalid.", nameof(value));
        var result = new byte[value.Length / 2];
        for (var index = 0; index < result.Length; index++)
            result[index] = Convert.ToByte(value.Substring(index * 2, 2), 16);
        return result;
    }

    private static string EncodeHex(byte[] value) =>
        string.Concat(value.Select(item => item.ToString("x2")));

    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase
    };

    public void Dispose()
    {
        _registry.Changed -= HandleRegistryChanged;
        _shutdown.Set();
        _snapshotChanged.Set();
        _worker?.Join(2000);
        _snapshotChanged.Dispose();
        _shutdown.Dispose();
    }
}

internal sealed class AgentInstallMarker
{
    public int SchemaVersion { get; set; }
    public uint AppId { get; set; }
    public string Runtime { get; set; } = string.Empty;
    public string Secret { get; set; } = string.Empty;

    public static AgentInstallMarker Read(string path)
    {
        if (string.IsNullOrWhiteSpace(path) || !File.Exists(path))
            throw new FileNotFoundException("The GameTweaks Agent marker was not found.", path);
        var marker = JsonSerializer.Deserialize<AgentInstallMarker>(
            File.ReadAllText(path), new JsonSerializerOptions { PropertyNameCaseInsensitive = true });
        if (marker is null || marker.SchemaVersion != 1 || marker.AppId == 0 || marker.Secret.Length != 64)
            throw new InvalidDataException("The GameTweaks Agent marker was invalid.");
        return marker;
    }
}
