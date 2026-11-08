import { readFileSync } from 'node:fs';

const workspace = readFileSync('Cargo.toml', 'utf8').match(/^version = "([^"]+)"/m)?.[1];
const pubspec = readFileSync('flutter/pubspec.yaml', 'utf8');
const flutter = pubspec.match(/^version:\s*([0-9]+\.[0-9]+\.[0-9]+)\+([0-9]+)$/m);
if (!workspace || !flutter || workspace !== flutter[1]) {
  throw new Error('Keep Cargo workspace and Flutter pubspec versions in sync.');
}
if (!/^\d+\.\d+\.\d+$/.test(workspace)) {
  throw new Error('Installer versions require major.minor.patch.');
}
const [major, minor, patch] = workspace.split('.').map(Number);
const expectedCode = major * 1000000 + minor * 10000 + patch * 100;
if (Number(flutter[2]) !== expectedCode) {
  throw new Error(`Flutter build number must be ${expectedCode}.`);
}
const gradle = readFileSync('flutter/android/app/build.gradle.kts', 'utf8');
if (!/minSdk\s*=\s*34/.test(gradle)) {
  throw new Error('Android minimum API must be 34.');
}
const tag = process.env.GITHUB_REF_TYPE === 'tag' ? process.env.GITHUB_REF_NAME : undefined;
if (tag && tag !== `v${workspace}`) {
  throw new Error(`Tag ${tag} does not match version ${workspace}.`);
}
const msix = `${major + 1}.${minor}.${patch}.0`;
console.log(`Release version: ${workspace}`);
console.log(`Flutter build number: ${expectedCode}`);
console.log(`Microsoft Store package version: ${msix}`);
