# Agent protocol v1

Transport is a byte-mode Windows Named Pipe named
`\\.\pipe\GameTweaks.Agent.v1`. Each frame consists of a four-byte
little-endian length followed by UTF-8 JSON. Frames are limited to 1 MiB.

1. The app sends `challenge` with 32 random bytes encoded as hexadecimal.
2. The Agent answers with `hello`. Its proof is
   `HMAC-SHA256(secret, challenge|appId|processId|instanceId)`.
3. The app verifies the pipe client PID, Steam executable path, per-game secret,
   protocol version and proof before sending `helloAck`.
4. The Agent sends bounded `snapshot` and `configChanged` messages.
5. The app may send only `setConfig`; the Agent answers with `configResult`.

`configResult.restartRequired` is set only after a deferred
`restartRequired` binding has safely persisted the value. `nextLaunch` values
are persisted without being applied to the current session.

There are no process, shell, arbitrary file, memory or generic RPC messages.
