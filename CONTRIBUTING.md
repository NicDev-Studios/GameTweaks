# Contributing to GameTweaks

GameTweaks uses Tauri 2, Rust, Svelte, TypeScript, and pnpm. Keep contributions modular, secure, and easy to review.

Use the checks documented in `AGENTS.md` for the area you change. Keep Rust responsible for configuration and updater state, keep Tauri commands thin, and keep desktop permissions minimal.

The interface should remain compact, neutral, and suitable for a desktop utility. Do not add fake feature data or broad placeholder dashboards.

## Branch workflow

`main` is the default integration branch and contains the newest reviewed development state. Stable versions are represented by immutable version tags and GitHub Releases, not by a separate permanent stable branch.

Create a short-lived branch from `main` using a descriptive prefix:

- `feature/<name>` for new functionality
- `fix/<name>` for bug fixes
- `docs/<name>` for documentation-only changes
- `chore/<name>` for maintenance work

Open the pull request against `main`. The `Publish Dev Build` CI job is skipped on pull requests. For pushes to `main`, it waits for the required CI checks and then updates the rolling `DEV_RELEASE` GitHub Prerelease for development-only testing. Failed or cancelled CI runs do not publish a development build. This tag is movable and must not be treated as a stable or reproducible version.

For a short beta cycle, prerelease tags such as `v1.0.0-beta.1` may be created directly from a tested commit on `main`. If a version needs a longer stabilization period while new work continues on `main`, create a temporary branch such as `release/1.0.0`. Publish its betas as `v1.0.0-beta.1`, `v1.0.0-beta.2`, and so on, then publish the stable `v1.0.0` tag from the tested release branch. Merge stabilization fixes back into `main` and delete the release branch after publication.
