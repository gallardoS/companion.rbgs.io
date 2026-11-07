import { spawnSync } from 'node:child_process';

const publicKey = process.env.RBGS_UPDATER_PUBLIC_KEY?.trim();
const privateKey = process.env.TAURI_SIGNING_PRIVATE_KEY?.trim();
if (Boolean(publicKey) !== Boolean(privateKey)) {
  throw new Error('Configure both RBGS_UPDATER_PUBLIC_KEY and TAURI_SIGNING_PRIVATE_KEY to enable updates.');
}
const args = ['node_modules/@tauri-apps/cli/tauri.js', 'build', '--bundles', 'nsis'];
if (publicKey) args.push('--config', JSON.stringify({ bundle: { createUpdaterArtifacts: true } }));
const result = spawnSync(process.execPath, args, { stdio: 'inherit', env: process.env });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
