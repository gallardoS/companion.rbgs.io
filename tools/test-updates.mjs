import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

test('release manifest pins the signed installer to its versioned release', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'rbgs-update-test-'));
  try {
    await mkdir(join(directory, 'release'));
    await writeFile(join(directory, 'package.json'), JSON.stringify({ version: '1.2.3' }));
    await writeFile(join(directory, 'release/rbgs.io-companion-setup.exe.sig'), 'test-signature\n');
    const result = spawnSync(process.execPath, [resolve('tools/prepare-update.mjs')], {
      cwd: directory, encoding: 'utf8',
      env: { ...process.env, RBGS_UPDATER_PUBLIC_KEY: 'test-public-key', GITHUB_REPOSITORY: 'owner/companion' },
    });
    assert.equal(result.status, 0, result.stderr);
    const manifest = JSON.parse(await readFile(join(directory, 'release/latest.json'), 'utf8'));
    assert.equal(manifest.version, '1.2.3');
    assert.deepEqual(manifest.platforms['windows-x86_64'], {
      signature: 'test-signature',
      url: 'https://github.com/owner/companion/releases/download/v1.2.3/rbgs.io-companion-setup.exe',
    });
    assert.ok(Number.isFinite(Date.parse(manifest.pub_date)));
    await rm(join(directory, 'release/rbgs.io-companion-setup.exe.sig'));
    const missing = spawnSync(process.execPath, [resolve('tools/prepare-update.mjs')], {
      cwd: directory, encoding: 'utf8', env: { ...process.env, RBGS_UPDATER_PUBLIC_KEY: 'test-public-key' },
    });
    assert.notEqual(missing.status, 0, 'A signed release must not silently omit its signature');
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('build rejects a public key without its signing key', () => {
  const result = spawnSync(process.execPath, ['tools/build-installer.mjs'], {
    encoding: 'utf8',
    env: { ...process.env, RBGS_UPDATER_PUBLIC_KEY: 'test-key', TAURI_SIGNING_PRIVATE_KEY: '' },
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Configure both/);
});

test('signed build supplies the public key to the Tauri bundler', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'rbgs-signed-build-test-'));
  try {
    await mkdir(join(directory, 'node_modules/@tauri-apps/cli'), { recursive: true });
    await writeFile(join(directory, 'node_modules/@tauri-apps/cli/tauri.js'),
      "require('node:fs').writeFileSync('build-args.json', JSON.stringify(process.argv.slice(2)));\n");
    const result = spawnSync(process.execPath, [resolve('tools/build-installer.mjs')], {
      cwd: directory, encoding: 'utf8',
      env: { ...process.env, RBGS_UPDATER_PUBLIC_KEY: 'test-public-key', TAURI_SIGNING_PRIVATE_KEY: 'test-private-key' },
    });
    assert.equal(result.status, 0, result.stderr);
    const args = JSON.parse(await readFile(join(directory, 'build-args.json'), 'utf8'));
    assert.deepEqual(args.slice(0, 3), ['build', '--bundles', 'nsis']);
    assert.equal(args[3], '--config');
    assert.deepEqual(JSON.parse(args[4]), {
      bundle: { createUpdaterArtifacts: true },
      plugins: { updater: { pubkey: 'test-public-key' } },
    });
    assert.ok(!args.join(' ').includes('test-private-key'));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
