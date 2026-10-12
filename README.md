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

## Local match history

Install and enable the product `RBGS` addon alongside the diagnostic addon. **Completed matches** reads the selected account's separate `RBGS.lua` / `RBGSMatchData` schema 2, currently limited to the controlled Darkspear/70338 adapter. Expand a completed capture to inspect its final roster and statistics. The winning faction is shown; player identity and match verification are not inferred from scoreboard names.

History refreshes every five seconds or through **Refresh match history**. Stable, bounded reads reuse unchanged data. The addon retains eight attempts; the companion preserves every observed revision transactionally in `capture-inbox-v1.sqlite3` under its Windows local-data directory. Saved captures survive restart, missing game files and clearing addon history.

Snapshots retain canonical UTF-8 JSON and SHA-256 digests. Identical revisions are deduplicated; conflicting contents are preserved and excluded from completed history. Codes and GUIDs remain private to native snapshots. Account context is confirmed through the native device API and isolated by API origin, credential fingerprint and selected game source. Unbound imports remain preserved separately.

The local queue is bounded to 100 snapshots, 25 MiB of payloads, 256 KiB per snapshot and 50 MiB disk headroom. Full, unavailable, corrupt or unsupported storage preserves existing files and reports an error. No snapshots are automatically pruned and no game files are written.

## Controlled capture delivery

Existing status-only devices must choose **Enable capture permissions** and approve assignment access and evidence uploads in web Settings. Rust checks current account and permissions before authenticated operations; tokens remain in Windows Credential Manager.

For local tests, enable `RBGS_CONTROLLED_CAPTURE_ENABLED=true` on the backend and restart it. An active moderator creates a controlled Darkspear test in web Settings. The companion lists this account's assignments. Issue the one-time code and paste the saved `/rbgs match RB2:…` command into WoW before entering the BG. Issuance is saved locally before display. If the code was issued but cannot be recovered locally or from the addon, request a fresh assignment.

After `/reload` or logout, terminal captures bound to an own assignment are uploaded every 15 seconds with durable retry delays and bounded `Retry-After` handling. The checkbox pauses automatic uploads; **Sync saved captures** triggers a manual attempt. A suspended webview can delay automatic synchronization.

SQLite schema 2 migrates the existing inbox and freezes the exact canonical HTTP body and SHA-256 digest before sending. Restarts and retries reuse those bytes. Receipt recovery runs before submission. **Received by server** requires matching assignment/capture IDs, revision, digest and durable-storage confirmation. Conflicts, damaged data and previous snapshots remain preserved. Issuances and transport records are bounded to 100 each.

Controlled reports always go to review. A server receipt confirms storage; it does not establish account ownership, verified match results or rating.
