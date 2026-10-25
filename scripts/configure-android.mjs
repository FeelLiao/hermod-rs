import { readFileSync, writeFileSync, existsSync, cpSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const android = path.join(root, 'hermod-app/src-tauri/gen/android');
const gradle = path.join(android, 'app/build.gradle.kts');
if (!existsSync(gradle)) throw new Error('Run npm run tauri -- android init --ci first.');
// 固定 API 36，与 CI 安装的 SDK 一致，避免生成器默认 SDK 升级后失配。
let build = readFileSync(gradle, 'utf8').replace(/compileSdk = \d+/, 'compileSdk = 36').replace(/targetSdk = \d+/, 'targetSdk = 36');
writeFileSync(gradle, build);
// 生成器通过 tauri.properties 同步 versionName/versionCode，不写入签名私钥。
const manifest = path.join(android, 'app/src/main/AndroidManifest.xml');
let xml = readFileSync(manifest, 'utf8');
if (!xml.includes('android.permission.INTERNET')) xml = xml.replace('<application', '<uses-permission android:name="android.permission.INTERNET"/>\n    <application');
writeFileSync(manifest, xml);
const icons = path.join(root, 'hermod-app/src-tauri/icons/android');
cpSync(icons, path.join(android, 'app/src/main/res'), { recursive: true });
console.log('Android project ready: native document picker, private inbox, foreground transfer service.');
