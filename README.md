# rbgs.io companion

The Windows companion for [rbgs.io](https://rbgs.io). Connect your account, select your WoW folder, and keep your match captures synced. Get notified when a test match is ready.

![rbgs.io companion Home screen](docs/images/companion-home.png)

## Download

Download **rbgs.io-companion-setup.exe** from the [latest release](https://github.com/gallardoS/companion.rbgs.io/releases/latest) and run the installer.

Open the companion, connect your account, and select your WoW folder. Install and enable the RBGS addon to capture your matches.

## Compile it yourself

On Windows, install [Node.js 22.12 or newer](https://nodejs.org/), [Rust 1.90 or newer](https://rustup.rs/), and the [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows), including Microsoft C++ Build Tools and WebView2.

Clone this repository and run the following commands from its folder.

```powershell
npm ci
npm run tauri -- build --bundles nsis
```

Your installer will be in `src-tauri/target/release/bundle/nsis/`.

Each successful build on `main` publishes a new numbered release with the Windows installer. You can also run **Build and release companion** manually from GitHub Actions on `main`.
