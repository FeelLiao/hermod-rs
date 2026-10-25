import { readFileSync, writeFileSync, existsSync, cpSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
const app = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const android = path.join(app, 'src-tauri/gen/android');
if (!existsSync(path.join(android, 'app/build.gradle.kts'))) throw new Error('Run tauri android init --ci first.');
const read = name => readFileSync(path.join(android, name), 'utf8');
const write = (name, text) => writeFileSync(path.join(android, name), text);
// Tauri 生成原生壳，版本和应用策略保存在 hermod-app；不修改依赖源码。
write('app/build.gradle.kts', readFileSync(path.join(app, 'android/app.build.gradle.kts'), 'utf8'));
for (const file of ['build.gradle.kts', 'buildSrc/build.gradle.kts']) {
  const text = read(file);
  if (!text.includes('com.android.tools.build:gradle:')) throw new Error(`Unknown Tauri template: ${file}`);
  write(file, text.replace(/com.android.tools.build:gradle:[\d.]+/g, 'com.android.tools.build:gradle:9.4.0'));
}
const rootGradle = read('build.gradle.kts');
if (!rootGradle.includes('// Hermod SDK policy')) write('build.gradle.kts', rootGradle + `
// Hermod SDK policy
subprojects {
    plugins.withId("com.android.library") {
        extensions.configure<com.android.build.gradle.LibraryExtension> {
            compileSdk = 37
            buildToolsVersion = "37.0.0"
            defaultConfig { minSdk = 34 }
        }
    }
}
`);
write('gradle/wrapper/gradle-wrapper.properties', `distributionBase=GRADLE_USER_HOME
 distributionPath=wrapper/dists
 distributionUrl=https\\://services.gradle.org/distributions/gradle-9.6.0-bin.zip
 distributionSha256Sum=bbaeb2fef8710818cf0e261201dab964c572f92b942812df0c3620d62a529a01
 networkTimeout=10000
 validateDistributionUrl=true
 zipStoreBase=GRADLE_USER_HOME
 zipStorePath=wrapper/dists
`.replace(/^ /gm, ''));
const config = JSON.parse(readFileSync(path.join(app, 'src-tauri/tauri.conf.json')));
write('app/tauri.properties', `tauri.android.versionCode=${config.bundle.android.versionCode}\ntauri.android.versionName=${config.version}\n`);
let manifest = read('app/src/main/AndroidManifest.xml');
if (!manifest.includes('android:extractNativeLibs=')) manifest = manifest.replace('<application', '<application android:extractNativeLibs="false"');
write('app/src/main/AndroidManifest.xml', manifest);
// Tauri 和官方插件目前采用 Kotlin 插件与旧 DSL，AGP 9.4 提供兼容开关。
let properties = read('gradle.properties');
for (const [key, value] of Object.entries({'android.builtInKotlin':'false','android.newDsl':'false','org.gradle.configuration-cache':'false'})) {
  properties = properties.replace(new RegExp(`^${key.replaceAll('.', '\\.')}=.*\\n?`, 'gm'), '');
  properties += `\n${key}=${value}\n`;
}
write('gradle.properties', properties);
// 生成器默认图标不代表 Hermod；使用应用自己的启动器资源。
cpSync(path.join(app, 'src-tauri/icons/android'), path.join(android, 'app/src/main/res'), { recursive: true });
console.log('Android shell configured: API 34+, target 37, Material Web UI, SAF and foreground transfers.');
