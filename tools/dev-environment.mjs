import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const origins = {
  local: 'http://localhost:8080',
  production: 'https://api.rbgs.io',
};
const environment = process.argv[2];
if (!Object.hasOwn(origins, environment)) {
  console.error('Choose local or production.');
  process.exit(1);
}

console.log(`Companion environment: ${environment} (${origins[environment]})`);
const child = spawn(process.execPath, [
  fileURLToPath(new URL('../node_modules/@tauri-apps/cli/tauri.js', import.meta.url)),
  'dev',
  ...process.argv.slice(3),
], {
  cwd: fileURLToPath(new URL('../', import.meta.url)),
  env: { ...process.env, RBGS_COMPANION_API_ORIGIN: origins[environment] },
  stdio: 'inherit',
  windowsHide: true,
});
child.on('error', () => {
  console.error('Could not start Tauri development. Check the installed dependencies.');
  process.exitCode = 1;
});
child.on('exit', (code) => { process.exitCode = code ?? 1; });
