import { readFile, writeFile, appendFile } from 'node:fs/promises';

const runNumber = Number(process.env.GITHUB_RUN_NUMBER);
if (!Number.isSafeInteger(runNumber) || runNumber < 1) {
  throw new Error('GITHUB_RUN_NUMBER must be a positive integer.');
}

const packagePath = 'package.json';
const packageLockPath = 'package-lock.json';
const tauriPath = 'src-tauri/tauri.conf.json';
const cargoPath = 'src-tauri/Cargo.toml';
const cargoLockPath = 'src-tauri/Cargo.lock';
const [packageText, packageLockText, tauriText, cargoText, cargoLockText] = await Promise.all(
  [packagePath, packageLockPath, tauriPath, cargoPath, cargoLockPath].map(path => readFile(path, 'utf8')),
);
const pkg = JSON.parse(packageText);
const packageLock = JSON.parse(packageLockText);
const tauri = JSON.parse(tauriText);
const baseVersion = /^(\d+)\.(\d+)\.(\d+)$/.exec(pkg.version);
if (!baseVersion) throw new Error('The package version must use major.minor.patch.');
const [major, minor, patch] = baseVersion.slice(1).map(Number);
const releasePatch = patch + runNumber - 1;
if ([major, minor, releasePatch].some(part => !Number.isSafeInteger(part) || part > 65535)) {
  throw new Error('The release version exceeds the Windows installer version limits.');
}
const version = `${major}.${minor}.${releasePatch}`;
const tag = `v${version}`;

function updateCargoVersion(source, pattern, label) {
  if (!pattern.test(source)) throw new Error(`Cannot find the companion version in ${label}.`);
  return source.replace(pattern, (_match, before, after) => `${before}${version}${after}`);
}

const cargo = updateCargoVersion(cargoText,
  /(^\[package\]\r?\nname = "rbgs-companion"\r?\nversion = ")[^"]+(")/m, cargoPath);
const cargoLock = updateCargoVersion(cargoLockText,
  /(^\[\[package\]\]\r?\nname = "rbgs-companion"\r?\nversion = ")[^"]+(")/m, cargoLockPath);
pkg.version = version;
packageLock.version = version;
packageLock.packages[''].version = version;
tauri.version = version;

await Promise.all([
  writeFile(packagePath, `${JSON.stringify(pkg, null, 2)}\n`),
  writeFile(packageLockPath, `${JSON.stringify(packageLock, null, 2)}\n`),
  writeFile(tauriPath, `${JSON.stringify(tauri, null, 2)}\n`),
  writeFile(cargoPath, cargo),
  writeFile(cargoLockPath, cargoLock),
]);
if (process.env.GITHUB_OUTPUT) {
  await appendFile(process.env.GITHUB_OUTPUT, `version=${version}\ntag=${tag}\n`);
}
console.log(`Release ${tag}`);
