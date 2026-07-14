# GameTweaks

GameTweaks is a cross-platform Tauri 2 desktop app for game customization. The current foundation provides the visual shell, settings, localization, theme handling, and signed updater flow while leaving product-specific features open for later work.

## Development

```sh
corepack enable
pnpm install
pnpm dev
```

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

The Svelte frontend lives in `src`, and the minimal settings/updater backend lives in `src-tauri`.
