# GameTweaks repository instructions

GameTweaks is a Tauri 2 desktop app built with Rust, Svelte, and TypeScript. The
Rust backend owns Steam discovery, Unity and BepInEx inspection, catalog trust,
downloads, filesystem transactions, process checks, and Agent IPC. The frontend
must treat backend results as authoritative and must not receive local game
paths, download URLs, or IPC secrets.

- Inspect the existing implementation and `AGENTS.md` before changing behavior.
- Keep changes narrow, typed, testable, and consistent with existing modules.
- Keep Tauri commands thin and capabilities minimal.
- Never add fake games, mods, status, versions, metrics, or release data.
- Never start games, execute user-controlled commands, change Steam launch
  options, bypass anti-cheat, or add arbitrary memory access.
- Preserve the conservative anti-cheat, process, path, archive, digest, marker,
  collision, staging, and rollback checks.
- Never trust catalog-provided download URLs. Mod downloads are constructed only
  from the fixed GameTweaks-Games GitHub release origin.
- Existing or manually installed game files must not be overwritten or removed.
- BepInEx, mods, the shared Agent, and configs must remain independently managed.
- Preserve unconfirmed frontend config drafts when Agent events arrive.
- Keep errors scoped to the affected game or mod and never display local paths or
  secrets.
- Preserve the neutral glass desktop style and accessible keyboard semantics.
- Add focused tests for changed parsers, validation, transactions, IPC, or config
  behavior. Run the checks documented in `AGENTS.md`.
