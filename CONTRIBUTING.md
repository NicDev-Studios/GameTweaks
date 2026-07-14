# Contributing to GameTweaks

GameTweaks uses Tauri 2, Rust, Svelte, TypeScript, and pnpm. Keep contributions modular, secure, and easy to review.

Use the checks documented in `AGENTS.md` for the area you change. Keep Rust responsible for configuration and updater state, keep Tauri commands thin, and keep desktop permissions minimal.

The interface should remain compact, neutral, and suitable for a desktop utility. Do not add fake feature data or broad placeholder dashboards.

## Branch workflow

`main` contains the current stable version of GameTweaks. Contributors should not open feature pull requests directly against `main`.

Create a short-lived branch from `development` using a descriptive prefix:

- `feature/<name>` for new functionality
- `fix/<name>` for bug fixes
- `docs/<name>` for documentation-only changes
- `chore/<name>` for maintenance work

Open the pull request against `development`. This branch is the integration stage for the next version: contributions are combined, reviewed, and tested there before they become stable. Beta versions are built from a tested `development` state and use prerelease versions and tags such as `0.1.0-beta.1` and `v0.1.0-beta.1`.

Once the beta has been tested and is ready for a stable release, `development` is merged into `main`. Stable release tags such as `v0.1.0` must be created from `main`. After a direct hotfix to `main`, merge the same change back into `development` so the branches do not diverge.
