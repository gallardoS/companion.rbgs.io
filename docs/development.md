# Companion development

## Build on Windows

Install [Node.js 22.12 or newer](https://nodejs.org/), [Rust 1.90 or newer](https://rustup.rs/), and the [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows), including Microsoft C++ Build Tools and WebView2.

Clone this repository and run these commands from its folder:

```powershell
npm ci
npm run tauri -- build --bundles nsis
```

The installer will be in `src-tauri/target/release/bundle/nsis/`.

## Releases and automatic updates

Each successful build on `main` publishes a new numbered release with the Windows installer. You can also run **Build and release companion** manually from GitHub Actions on `main`.

Signed release builds check the latest GitHub release when the companion starts. A newer version is downloaded and its signature verified before the Windows installer takes over and reopens the companion. Development and unsigned builds do not update themselves.

To enable signing in GitHub Actions, generate a signing key:

```powershell
npm run tauri -- signer generate -w <secure-path-outside-the-repository>
```

Keep a protected backup of the private key: existing installations trust its public key. Reuse the same key for subsequent releases.

Configure these repository settings:

- Actions variable `RBGS_UPDATER_PUBLIC_KEY`: contents of the generated `.pub` file.
- Actions secret `TAURI_SIGNING_PRIVATE_KEY`: contents of the private key file.
- Actions secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the key password, if set.

The workflow signs the installer and publishes its `.sig` and `latest.json` alongside the existing downloads. Both keys must be configured together; with neither configured, the workflow produces manual installers.

For local signed builds, set the same environment variables and run:

```powershell
node tools/build-installer.mjs
```

Users of versions released before the updater must install an enabled release manually once. Subsequent newer releases update automatically on startup.
