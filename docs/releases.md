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

## Publishing

Create and push a SemVer tag:

```sh
git tag v0.1.1
git push origin v0.1.1
```

The release workflow builds macOS Apple Silicon, macOS Intel, Windows, and
Linux bundles, signs updater artifacts, and attaches `latest.json` to the
GitHub Release. macOS is built per architecture so the updater manifest contains
the `darwin-aarch64` and `darwin-x86_64` platform entries expected by packaged
apps. Packaged GameTweaks builds check:

```text
https://github.com/NicDevTV/GameTweaks/releases/latest/download/latest.json
```
