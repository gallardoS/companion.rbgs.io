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

## Saved game data

The companion checks `C:\Program Files (x86)\World of Warcraft` and `C:\Program Files\World of Warcraft` for the Forever beta client. A saved installation takes precedence. Use **Scan Folders** for another drive or games folder, or choose `_classic_beta_` directly. Scanning skips junctions and symlinks and is bounded to six folder levels, 10,000 directories, 100,000 entries and ten seconds between filesystem operations.

Install and enable `RBGSCompatibility` (interface 16001). Choose the account source explicitly when multiple sources exist. Use `/reload` or log out in WoW; the open companion reads changes automatically. The selected installation and account persist across restarts.

Only the selected diagnostic SavedVariables file is read. One-second checks compare bounded bytes and modification time, wait for matching samples, then verify stability before parsing. Invalid or removed files clear the previous summary. A suspended webview can delay checks. **Refresh saved data** remains available.

The data-only Lua reader executes no Lua and rejects unsafe paths, duplicate keys, unknown schemas and executable content. Limits: 2,000,000 bytes, 30 nesting levels, 200,000 entries, 300 records and 16,384 bytes per string. Unchecked client builds are marked explicitly. Saved observations are not verified match evidence. No game files are modified or uploaded.

The diagnostic Python reader and sanitized fixtures are in `tools/`:

```powershell
python -m unittest discover -s tools -p test_parse_probe.py
```
