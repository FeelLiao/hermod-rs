import { readFileSync } from 'node:fs';
const workspace = readFileSync('Cargo.toml', 'utf8').match(/^version = "([^"]+)"/m)?.[1];
const frontend = JSON.parse(readFileSync('hermod-app/package.json')).version;
const native = JSON.parse(readFileSync('hermod-app/src-tauri/tauri.conf.json')).version;
if (!workspace || workspace !== frontend || workspace !== native) throw new Error('Keep workspace, package.json and tauri.conf.json versions in sync.');
if (!/^\d+\.\d+\.\d+$/.test(workspace)) throw new Error('Installer versions currently require major.minor.patch.');
const tag = process.env.GITHUB_REF_TYPE === 'tag' ? process.env.GITHUB_REF_NAME : undefined;
if (tag && tag !== `v${workspace}`) throw new Error(`Tag ${tag} does not match version ${workspace}.`);
console.log(`Release version: ${workspace}`);

const android = JSON.parse(readFileSync('hermod-app/src-tauri/tauri.conf.json')).bundle.android;
const [major, minor, patch] = workspace.split('.').map(Number);
if (android.minSdkVersion !== 34) throw new Error('Android minimum API must be 34.');
if (android.versionCode !== major * 1000000 + minor * 10000 + patch * 100) throw new Error('Update Android versionCode with each release.');
