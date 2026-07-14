# Contributing to GameTweaks

GameTweaks uses Tauri 2, Rust, Svelte, TypeScript, and pnpm. Keep contributions modular, secure, and easy to review.

Use the checks documented in `AGENTS.md` for the area you change. Keep Rust responsible for configuration and updater state, keep Tauri commands thin, and keep desktop permissions minimal.

The interface should remain compact, neutral, and suitable for a desktop utility. Do not add fake feature data or broad placeholder dashboards.
