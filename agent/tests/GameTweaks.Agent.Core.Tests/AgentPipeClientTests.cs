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
}
