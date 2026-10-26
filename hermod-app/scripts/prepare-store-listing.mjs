// 使用真实共享界面及隔离的演示 IPC 截图；不会连接信令服务器或读取用户数据。
import { chromium } from '@playwright/test';
import { createServer } from 'vite';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';

const app = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const repository = path.dirname(app);
const source = path.join(app, 'store-listing');
const version = JSON.parse(await readFile(path.join(app, 'package.json'), 'utf8')).version;
const listing = JSON.parse(await readFile(path.join(source, 'listing.json'), 'utf8'));
const output = path.join(repository, 'store-output', `hermod-${version}`);
await mkdir(output, { recursive: true });
const outputs = [];
async function put(name, content) {
  const destination = path.join(output, name);
  await mkdir(path.dirname(destination), { recursive: true });
  await writeFile(destination, content);
  outputs.push(name);
}
const escape = text => text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('"', '&quot;');
const images = [];
const viewport = { width: 1600, height: 900 };
const server = await createServer({ root: app, server: { host: '127.0.0.1', port: 0 }, logLevel: 'error' });
let browser;
try {
  await server.listen();
  const address = server.httpServer.address();
  const baseURL = `http://127.0.0.1:${address.port}`;
  browser = await chromium.launch({ headless: true });
  for (const [locale, copy] of Object.entries(listing.locales)) {
    assert(copy.description.length <= 10000 && copy.shortDescription.length <= 270);
    assert(copy.updateNotes.length <= 1500 && copy.features.length <= 20);
    assert(copy.features.every(text => text.length <= 200));
    assert(copy.systemRequirements.every(text => text.length <= 200));
    assert(copy.screenshots.length >= 4 && copy.screenshots.length <= 10);
    assert(copy.screenshots.every(item => item.caption.length <= 200));
    const textFiles = {
      'description.txt': copy.description,
      'short-description.txt': copy.shortDescription,
      'features.txt': copy.features.join('\n'),
      'system-requirements.txt': copy.systemRequirements.join('\n'),
      'whats-new-for-updates.txt': copy.updateNotes,
      'screenshot-captions.txt': copy.screenshots.map(item => `${item.file}\n${item.caption}`).join('\n\n'),
    };
    for (const [name, content] of Object.entries(textFiles)) await put(`copy/${locale}/${name}`, `${content}\n`);
    const chinese = locale === 'zh-CN';
    const sample = Buffer.from((chinese ? '# 项目笔记\n\n会议记录、行动事项与交付计划。\n' : '# Project notes\n\nMeeting notes, action items and delivery plans.\n').repeat(256));
    const sampleName = chinese ? '项目笔记.md' : 'Project notes.md';
    const metadata = (name, content) => ({
      id: createHash('sha256').update(name).digest('hex').slice(0, 32),
      name, size: content.length, sha256: createHash('sha256').update(content).digest('hex'),
    });
    const inbox = [
      metadata(sampleName, sample),
      metadata(chinese ? '出行清单.txt' : 'Travel checklist.txt', Buffer.from('Documents\nCharger\nTravel plans\n'.repeat(40))),
      metadata(chinese ? '项目计划.csv' : 'Project schedule.csv', Buffer.from('task,status\nPlanning,complete\nReview,ready\n'.repeat(30))),
    ];
    for (const [index, shot] of copy.screenshots.entries()) {
      const context = await browser.newContext({ viewport, deviceScaleFactor: 1.2, locale, colorScheme: index === 2 ? 'dark' : 'light' });
      await context.route('**/*', route => new URL(route.request().url()).origin === baseURL ? route.continue() : route.abort());
      await context.addInitScript(({ locale, dark, version, sampleName, sampleSize, inbox }) => {
        const server = 'wss://oracle.stariverfeel.eu.org:4376';
        let settings = {
          connection: { transport: 'wireguard', mtu: 'auto', congestion: 'auto', limit_mbps: '8' },
          receive_transport: 'auto', server, words: 3, verbose: 'none', quiet: false,
          theme: dark ? 'dark' : 'light', language: locale, destination: null,
        };
        let status = null;
        window.__TAURI_INTERNALS__ = {
          invoke: async (command, args) => {
            switch (command) {
              case 'bootstrap': return {
                version, ipv6: ['2001:db8::42'], platform: 'windows', settings: structuredClone(settings),
                settings_saved: true, trusted_servers: { [server]: 'demo-fingerprint-not-for-connections' },
              };
              case 'set_locale': return;
              case 'save_settings': settings = structuredClone(args.settings); return;
              case 'status': return status;
              case 'choose_source': return { id: 'demo-source', name: sampleName, size: sampleSize };
              case 'start_transfer': status = {
                phase: 'Waiting for the receiver', code: '2468-acid-acorn-acre', bytes: 0, total: sampleSize,
                elapsed: 0, done: false, cancelled: false, error: null, digest: null,
                rtt_ms: null, transport: null, received_id: null,
              }; return;
              case 'list_received': return inbox;
              default: throw Error(`Unexpected store capture command: ${command}`);
            }
          },
        };
      }, { locale, dark: index === 2, version, sampleName, sampleSize: sample.length, inbox });
      const page = await context.newPage();
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.goto(baseURL, { waitUntil: 'networkidle' });
      const button = (en, zh) => page.getByRole('button', { name: chinese ? zh : en, exact: true });
      const tab = (en, zh) => page.getByRole('tab', { name: chinese ? zh : en, exact: true });
      const field = (en, zh) => page.getByLabel(chinese ? zh : en, { exact: true });
      await button('Create transfer code', '生成传输码').waitFor();
      if (index === 0 || index === 1) {
        await page.getByRole('button', { name: chinese ? /选择文件/ : /Choose file/ }).click();
        await page.getByText(sampleName, { exact: true }).waitFor();
        if (index === 1) {
          await button('Create transfer code', '生成传输码').click();
          await page.getByTestId('transfer-code').waitFor();
        }
      } else if (index === 2) {
        await tab('Receive', '接收').click();
        await field('Transfer code', '传输码').fill('2468-acid-acorn-acre');
      } else if (index === 3) {
        await tab('Text', '文本').click();
        await field('Message', '文本内容').fill(chinese
          ? '你好！\n\n这是明天的讨论提纲：\n1. 确认项目目标与交付时间\n2. 分享设计方案和参考资料\n3. 整理下一步的行动清单\n\n通过 Hermod 把这段内容直接发给另一台设备。'
          : 'Hello!\n\nHere is the agenda for tomorrow:\n1. Confirm the project goals and delivery date\n2. Share the design proposal and references\n3. Agree on the next steps\n\nSend this note directly to another device with Hermod.');
      } else if (index === 4) {
        await tab('Inbox', '收件箱').click();
        await page.getByText(sampleName, { exact: true }).waitFor();
      } else {
        await tab('Settings', '设置').click();
        await field('Send protocol', '发送协议').selectOption('quic');
        await button('Advanced network settings', '高级网络设置').click();
        await field('MTU', 'MTU').fill('1280');
        await field('Congestion control', '拥塞控制').selectOption('bbr');
        await page.evaluate(() => window.scrollTo(0, 0));
      }
      await page.evaluate(() => { document.activeElement?.blur(); return document.fonts.ready; });
      await page.mouse.move(0, 0);
      await page.waitForTimeout(350);
      assert.deepEqual(errors, [], `Console errors in ${locale}/${shot.file}`);
      assert.equal(await page.locator('html').getAttribute('lang'), locale);
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Unexpected horizontal overflow');
      const name = `screenshots/${locale}/${shot.file}`;
      await put(name, await page.screenshot({ type: 'png', animations: 'disabled', caret: 'hide' }));
      images.push({ name, width: 1920, height: 1080, caption: shot.caption });
      await context.close();
    }
  }

  // 图标从现有 SVG 渲染；宣传图是独立矢量素材，不编辑或重绘应用截图。
  const icon = await readFile(path.join(app, 'icon.svg'), 'utf8');
  const hero = await readFile(path.join(source, 'hero.svg'), 'utf8');
  async function renderSVG(svg, width, height, name, transparent) {
    const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
    await page.setContent(`<html><head><style>html,body{margin:0;width:100%;height:100%;background:transparent}svg{display:block;width:100%;height:100%}</style></head><body>${svg}</body></html>`);
    await put(name, await page.screenshot({ type: 'png', omitBackground: transparent }));
    images.push({ name, width, height, caption: name });
    await page.close();
  }
  await renderSVG(icon, 300, 300, 'logos/app-tile-300.png', true);
  await renderSVG(icon, 1024, 1024, 'logos/app-icon-1024.png', true);
  await renderSVG(hero, 1920, 1080, 'promotional/hero-1920x1080.png', false);
  await put('sources/app-icon.svg', icon);
  await put('sources/hero.svg', hero);
  await put('UPLOAD.zh-CN.md', await readFile(path.join(source, 'UPLOAD.zh-CN.md')));
  await put('listing.json', `${JSON.stringify(listing, null, 2)}\n`);
  await put('PREVIEW.html', `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Hermod Store Listing Preview</title><style>body{margin:0;padding:40px;font:16px/1.6 system-ui,sans-serif;background:#f1f6f4;color:#12352f}main{max-width:1360px;margin:auto}h1{font-size:36px}section{display:grid;grid-template-columns:repeat(auto-fit,minmax(400px,1fr));gap:24px}figure{margin:0;background:white;padding:16px;border-radius:12px;overflow:hidden}img{display:block;width:100%;height:auto;border:1px solid #e1ebe7}figcaption{padding-top:12px}code{font-size:12px;overflow-wrap:anywhere}</style><main><h1>Hermod · 商店素材预览</h1><p>版本 ${escape(version)} · 实际桌面界面与演示数据 · 上传位置见 UPLOAD.zh-CN.md</p><section>${images.map(item => `<figure><a href="${item.name}"><img src="${item.name}" alt="${escape(item.caption)}"></a><figcaption>${escape(item.caption)}<br><code>${item.name} · ${item.width}×${item.height}</code></figcaption></figure>`).join('')}</section></main></html>`);
  for (const item of images) {
    const bytes = await readFile(path.join(output, item.name));
    assert(bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])), 'Not a PNG');
    assert.equal(bytes.readUInt32BE(16), item.width, item.name);
    assert.equal(bytes.readUInt32BE(20), item.height, item.name);
    assert(bytes.length < 50 * 1024 * 1024, 'Image exceeds Store upload limit');
  }
  await put('asset-manifest.json', `${JSON.stringify({
    version, storeId: listing.storeId, screenshots: 'Rendered shared desktop UI with isolated demonstration IPC; not Windows-native captures or live performance measurements.',
    images,
  }, null, 2)}\n`);
  // ZIP 只包含这次生成的明确文件清单，旧目录残留或用户文件不会被打包。
  const zipPath = `${output}-store-listing.zip`;
  execFileSync('python3', ['-c', 'import json,pathlib,sys,zipfile; root=pathlib.Path(sys.argv[1]); names=json.load(sys.stdin); z=zipfile.ZipFile(sys.argv[2],"w",zipfile.ZIP_DEFLATED); [(z.write(root/name,name)) for name in names]; z.close()', output, zipPath], { input: JSON.stringify(outputs) });
  console.log(`Prepared ${images.length} validated PNG images and bilingual copy: ${output}`);
  console.log(`Upload kit: ${zipPath}`);
} finally {
  await browser?.close();
  await server.close();
}
