import { readFileSync } from 'node:fs';

const packageVersion = JSON.parse(readFileSync('package.json', 'utf8')).version;
const tauriVersion = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')).version;
const cargoToml = readFileSync('src-tauri/Cargo.toml', 'utf8');
const packageStart = cargoToml.indexOf('[package]');
const packageEnd = cargoToml.indexOf('\n[', packageStart + '[package]'.length);
const cargoPackage = packageStart >= 0
  ? cargoToml.slice(packageStart, packageEnd >= 0 ? packageEnd : undefined)
  : '';
const cargoVersion = cargoPackage.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

if (!cargoVersion) {
  throw new Error('Could not read [package].version from src-tauri/Cargo.toml');
}

const versions = {
  'package.json': packageVersion,
  'src-tauri/Cargo.toml': cargoVersion,
  'src-tauri/tauri.conf.json': tauriVersion,
};

const uniqueVersions = new Set(Object.values(versions));
if (uniqueVersions.size !== 1) {
  const details = Object.entries(versions).map(([file, version]) => `${file}: ${version}`).join('\n');
  throw new Error(`Application versions are out of sync:\n${details}`);
}

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(packageVersion)) {
  throw new Error(`Application version is not valid SemVer: ${packageVersion}`);
}

const tag = process.env.GITHUB_REF_TYPE === 'tag' ? process.env.GITHUB_REF_NAME : undefined;
if (tag && tag !== `v${packageVersion}`) {
  throw new Error(`Release tag ${tag} does not match application version v${packageVersion}`);
}

console.log(`AutoForge version ${packageVersion} is consistent.`);
