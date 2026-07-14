# Agent Instructions

GameTweaks is a Tauri 2 desktop app built with Rust, Svelte, and TypeScript.

## Working rules

- Inspect relevant files before changing behavior.
- Keep changes narrow and do not add fake product data.
- Keep settings and updater code independent from future tweak features.
- Keep Tauri commands thin and permissions minimal.
- Preserve the neutral glass desktop style.
- Do not add process execution or server-management functionality unless explicitly requested.

## Checks

```sh
CI=true pnpm check
CI=true pnpm lint
CI=true pnpm build

cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```
