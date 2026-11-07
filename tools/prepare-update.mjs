import { readFile, writeFile } from 'node:fs/promises';

if (process.env.RBGS_UPDATER_PUBLIC_KEY?.trim()) {
  const { version } = JSON.parse(await readFile('package.json', 'utf8'));
  const signature = (await readFile('release/rbgs.io-companion-setup.exe.sig', 'utf8')).trim();
  if (!signature) throw new Error('Missing updater signature.');
  const repository = process.env.GITHUB_REPOSITORY;
  if (!repository) throw new Error('Missing release repository.');
  await writeFile('release/latest.json', JSON.stringify({
    version,
    pub_date: new Date().toISOString(),
    platforms: {
      'windows-x86_64': {
        signature,
        url: `https://github.com/${repository}/releases/download/v${version}/rbgs.io-companion-setup.exe`,
      },
    },
  }, null, 2) + '\n');
}
