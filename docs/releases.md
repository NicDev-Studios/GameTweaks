# Releases

GameTweaks updates are distributed through signed Tauri updater artifacts attached
to GitHub Releases.

## Signing setup

Generate a Tauri updater key pair locally:

```sh
pnpm tauri signer generate -w ~/.tauri/gametweaks.key
```

Copy the public key into `src-tauri/tauri.conf.json` as
`plugins.updater.pubkey`.

Add the private key to GitHub Actions secrets:

- `TAURI_SIGNING_PRIVATE_KEY`: the private key file content or path used by CI
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: optional, only if the key has a password

Never commit the private key.

## Versioning

The checked-in project version is the non-release placeholder `0.0.0-dev`.
Local builds, including `pnpm dev`, display `DEV_WORKING` and do not query the
automatic updater.

The release workflow is the only source of distributable versions. A version
tag such as `v1.0.0-beta.2` produces application and updater artifacts with
version `1.0.0-beta.2`. Rolling `DEV_RELEASE` builds receive a unique CI version
such as `0.0.0-dev.123`, but remain excluded from the application updater.

## Publishing

### Development builds

Every push to `main` updates the `DEV_RELEASE` tag and its GitHub Prerelease.
The release is rebuilt for every supported platform and is intended only for
testing the newest reviewed development state. Its permanent release page is:

```text
https://github.com/NicDev-Studios/GameTweaks/releases/tag/DEV_RELEASE
```

`DEV_RELEASE` is a movable tag. Do not use it as a stable or reproducible
version reference. It is excluded from both application update channels and is
available only as a manual test download.

### Beta and stable releases

Create and push a SemVer prerelease tag for a deliberate beta:

```sh
git tag v1.0.0-beta.1
git push origin v1.0.0-beta.1
```

Tags containing a SemVer prerelease suffix are published as GitHub
Prereleases. Create the final stable tag after the version has been tested:

```sh
git tag v1.0.0
git push origin v1.0.0
```

The release workflow builds macOS Apple Silicon, macOS Intel, Windows, and
Linux bundles, signs updater artifacts, and attaches `latest.json` to the
GitHub Release. macOS is built per architecture so the updater manifest contains
the `darwin-aarch64` and `darwin-x86_64` platform entries expected by packaged
apps. Packaged GameTweaks builds check:

```text
https://github.com/NicDev-Studios/GameTweaks/releases/latest/download/latest.json
```

For a longer beta cycle, branch `release/1.0.0` from the selected `main`
commit. Create beta and stable tags from that branch, merge stabilization fixes
back into `main`, and delete the release branch after the stable publication.
