# rbgs.io companion

Windows desktop companion built with Tauri 2, Rust and React. Device pairing uses an opaque scoped API credential stored in Windows Credential Manager. Tokens stay in native Rust code; the webview receives connection status only.

## Local development

Requires Node.js >= 22.12, Rust >= 1.90, Microsoft C++ Build Tools and WebView2. Start the backend on `http://localhost:8080` and the rbgs.io web app on `http://localhost:5173`.

```powershell
npm ci
$env:RBGS_COMPANION_API_ORIGIN = "http://localhost:8080"
npm run tauri -- dev
```

The API override is read at compile time and accepted only in debug builds. Release builds use `https://api.rbgs.io`.

Generate a pairing code and approve it in web Settings while signed in. The companion polls every five seconds and stores the issued credential. Capture permissions require a separate explicit approval. Revoke devices in web Settings.

## Verification and build

```powershell
npm run build
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run tauri -- build
```

The Windows NSIS installer and executable use the rbgs.io icon.
