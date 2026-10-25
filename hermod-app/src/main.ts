import '@fontsource/roboto/400.css';
import '@fontsource/roboto/500.css';
import '@fontsource/roboto/700.css';
import '@material/web/button/filled-button.js';
import '@material/web/button/outlined-button.js';
import '@material/web/button/text-button.js';
import '@material/web/textfield/outlined-text-field.js';
import '@material/web/progress/linear-progress.js';
import { invoke } from '@tauri-apps/api/core';
import './style.css';

type View = 'transfer' | 'inbox' | 'settings';
type Field = HTMLElement & { value: string; disabled: boolean; error: boolean; errorText: string };
type Button = HTMLElement & { disabled: boolean };
interface Selection { id: string; name: string; size: number | null }
interface Received { id: string; name: string; size: number; sha256: string }
interface Status { phase: string; code: string | null; bytes: number; total: number | null; elapsed: number;
  done: boolean; cancelled: boolean; error: string | null; digest: string | null; rtt_ms: number | null }
interface Bootstrap { version: string; server: string; ipv6: string[]; platform: string }
const paths: Record<string, string> = {
  transfer: 'M4 8h14m-4-4 4 4-4 4M20 16H6m4-4-4 4 4 4',
  inbox: 'M4 4h16v16H4zM4 14h5l2 3h2l2-3h5',
  settings: 'M4 6h16M4 12h16M4 18h16M8 3v6m8 0v6m-6 0v6',
  send: 'm4 4 17 8-17 8 3-8-3-8Zm3 8h14',
  receive: 'M12 3v12m-5-5 5 5 5-5M4 15v6h16v-6',
  file: 'M6 3h8l4 4v14H6zM14 3v5h4M9 12h6m-6 4h6',
  shield: 'm12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6zM8 12l3 3 5-6',
  arrow: 'M5 12h14m-6-6 6 6-6 6',
  copy: 'M9 8h11v13H9zM15 8V3H4v13h5',
};
const icon = (name: string, slot = '') => `<svg ${slot ? `slot="${slot}"` : ''} class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${paths[name] ?? paths.file}"/></svg>`;
const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const escape = (s: string) => s.replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
const size = (n: number | null) => n === null ? 'Size supplied by document provider' : n < 1024 ? `${n} B` : n < 1048576 ? `${(n/1024).toFixed(1)} KB` : n < 1073741824 ? `${(n/1048576).toFixed(1)} MB` : `${(n/1073741824).toFixed(2)} GB`;
let selected: Selection | null = null;
let running = false;
let busy = false;
let mode: 'send' | 'receive' = 'send';
let lastStatus: Status | null = null;
let previousDone = false;
let view: View = 'transfer';
let files: Received[] = [];

$('app').innerHTML = `
  <nav class="navigation" aria-label="Main navigation">
    <div class="brand-mark" aria-label="Hermod">H<span>↗</span></div>
    ${(['transfer','inbox','settings'] as const).map(v => `<button class="nav-item ${v === 'transfer' ? 'selected' : ''}" data-view="${v}" aria-current="${v === 'transfer' ? 'page' : 'false'}"><span class="nav-icon">${icon(v)}</span><span>${v === 'transfer' ? 'Transfer' : v === 'inbox' ? 'Inbox' : 'Settings'}</span></button>`).join('')}
    <span class="rail-caption">DIRECT<br/>BY DESIGN</span>
  </nav>
  <div class="shell">
    <header><div class="wordmark">hermod<span> / </span><span class="header-page" id="header-page">Transfer</span></div><span class="connection-badge" id="connection-badge"><span class="dot"></span>Checking network</span></header>
    <main>
      <section id="page-transfer" class="page">
        <div class="page-heading"><div class="eyebrow">YOUR FILES. YOUR CONNECTION.</div><h1>A direct connection.<br/><span>A little more freedom.</span></h1><p>Send something big. Keep it between you.</p></div>
        <div class="transfer-grid">
          <section class="card compose" aria-labelledby="compose-title">
            <div class="segmented" role="group" aria-label="Transfer direction"><button id="mode-send" aria-pressed="true">${icon('send')}Send</button><button id="mode-receive" aria-pressed="false">${icon('receive')}Receive</button></div>
            <div id="send-form"><h2 id="compose-title">What are we sending?</h2><p class="support">One file. A short code. Straight to your peer.</p>
              <button id="choose-file" class="file-picker" aria-label="Choose file"><span class="file-symbol">${icon('file')}</span><strong id="file-name">Choose a file</strong><span id="file-size">Browse files on this device</span><span class="picker-action">Open file picker ${icon('arrow')}</span></button>
            </div>
            <div id="receive-form" hidden><h2>Ready when they are.</h2><p class="support">Enter the code shared by your sender.</p><md-outlined-text-field id="receive-code" label="Transfer code" placeholder="1234-word-word-word" autocomplete="off" spellcheck="false"></md-outlined-text-field><div class="receive-note">${icon('inbox')}<span>Verified files land in your private inbox. Save a copy anywhere when you are ready.</span></div></div>
            <div class="limit-row"><span>${icon('settings')}<span>Bandwidth budget<br/><small id="limit-summary">8 Mbps payload cap</small></span></span><md-text-button id="edit-budget">Adjust</md-text-button></div>
            <md-filled-button id="start-transfer">${icon('send','icon')}Create transfer code</md-filled-button>
            <p class="fineprint">Both devices need reachable IPv6. Keep the code private.</p>
          </section>
          <div class="side-column">
            <section class="card status-card" aria-labelledby="status-title"><div class="section-label"><span class="dot" id="status-dot"></span><span id="status-label">READY TO CONNECT</span><span class="tag">P2P</span></div><h2 id="status-title">A clear path to your peer.</h2><p class="support" id="status-detail">Your file goes directly between devices. The signaling server only introduces you.</p>
              <div class="code-panel" id="code-panel" hidden><span>SHARE THIS CODE</span><code id="transfer-code"></code><md-outlined-button id="copy-code">${icon('copy','icon')}Copy code</md-outlined-button></div>
              <div id="progress-panel" hidden><md-linear-progress id="progress" aria-label="Transfer progress"></md-linear-progress><div class="progress-labels"><span id="progress-bytes"></span><span id="progress-percent"></span></div><div class="metrics"><div><strong id="speed">—</strong><span>avg. payload</span></div><div><strong id="rtt">—</strong><span>network RTT</span></div></div></div>
              <div class="digest" id="digest-panel" hidden><span>SHA-256 VERIFIED</span><code id="digest"></code></div>
              <md-outlined-button id="cancel-transfer" hidden>Cancel transfer</md-outlined-button>
              <div class="connection-art" id="connection-art" aria-hidden="true"><div>${icon('file')}</div><span>· · · · ·</span><div>${icon('shield')}</div><span>· · · · ·</span><div>${icon('receive')}</div></div>
            </section>
            <aside class="privacy-note">${icon('shield')}<div><strong>Private from the first byte</strong><p>Encrypted peer to peer. Every completed file is checked with SHA-256.</p></div></aside>
          </div>
        </div>
        <div id="network-warning" class="notice" hidden>No usable IPv6 address was detected. Connect to an IPv6 network before starting. WSL requires working IPv6 routing on the host and guest.</div>
      </section>
      <section id="page-inbox" class="page" hidden><div class="page-heading"><div class="eyebrow">ARRIVED. VERIFIED. YOURS.</div><h1>Your inbox.</h1><p>Keep a private copy, or save it where you need it.</p></div><div class="inbox-toolbar"><span id="inbox-count">0 files</span><md-text-button id="refresh-inbox">Refresh</md-text-button></div><div id="inbox-list"></div><p class="fineprint">Uninstalling Hermod removes its private inbox. Export files you want to keep.</p></section>
      <section id="page-settings" class="page" hidden><div class="page-heading"><div class="eyebrow">MAKE ROOM FOR EVERYTHING ELSE.</div><h1>Your connection.<br/><span>Your pace.</span></h1><p>A little upload headroom makes the whole network feel better.</p></div><div class="settings-grid"><section class="card"><h2>Bandwidth budget</h2><p class="support">Start at 70–85% of your measured upload speed. A full upload queue can delay browsing and calls.</p><md-outlined-text-field id="bandwidth" label="Payload limit (Mbps)" type="number" min="0" step="0.1" value="8" supporting-text="0 = unlimited. Applies to the next transfer."></md-outlined-text-field><md-filled-button id="save-settings">Save budget</md-filled-button><p class="fineprint">This cap excludes encryption and retransmission overhead. The lower of the two peers’ budgets is used by updated senders.</p></section><section class="card"><h2>Connection details</h2><dl><dt>Transport</dt><dd>QUIC · TLS 1.3 · IPv6</dd><dt>Signaling server</dt><dd id="server-address">Loading…</dd><dt>Local IPv6 addresses</dt><dd id="ipv6-addresses">Loading…</dd><dt>Application</dt><dd id="app-version">Hermod</dd></dl><label class="theme-choice" for="theme">Appearance<select id="theme"><option value="system">System</option><option value="light">Light</option><option value="dark">Dark</option></select></label></section></div></section>
    </main>
    <footer><span>Hermod</span><span>No file relay. No cloud storage.</span></footer>
  </div>
  <div id="snackbar" role="status" aria-live="polite" hidden></div>
  <dialog id="delete-dialog"><h2>Remove this inbox copy?</h2><p>Copies you have already exported will stay on your device.</p><div class="dialog-actions"><md-text-button id="delete-cancel">Keep file</md-text-button><md-filled-button id="delete-confirm">Remove copy</md-filled-button></div></dialog>
`;

let noticeTimer: ReturnType<typeof setTimeout>;
function notify(message: string, error = false) {
  clearTimeout(noticeTimer);
  $('snackbar').textContent = message;
  $('snackbar').classList.toggle('error', error);
  $('snackbar').hidden = false;
  noticeTimer = setTimeout(() => { $('snackbar').hidden = true; }, error ? 12000 : 5000);
}
function navigate(next: View) {
  view = next;
  for (const v of ['transfer','inbox','settings']) $(`page-${v}`).hidden = v !== next;
  document.querySelectorAll<HTMLElement>('[data-view]').forEach(el => {
    el.classList.toggle('selected', el.dataset.view === next);
    el.setAttribute('aria-current', el.dataset.view === next ? 'page' : 'false');
  });
  $('header-page').textContent = next === 'transfer' ? 'Transfer' : next === 'inbox' ? 'Inbox' : 'Settings';
  if (next === 'inbox') void refreshInbox();
  window.scrollTo({ top: 0, behavior: 'smooth' });
}
function controls() {
  const locked = busy || running;
  for (const id of ['choose-file','mode-send','mode-receive','receive-code','edit-budget']) $(id).toggleAttribute('disabled', locked);
  $('start-transfer').toggleAttribute('disabled', locked || (mode === 'send' && !selected));
  $('start-transfer').textContent = busy ? 'Working…' : running ? 'Transfer in progress' : mode === 'send' ? 'Create transfer code' : 'Connect & receive';
  document.querySelectorAll<Button>('[data-export],[data-delete]').forEach(b => b.disabled = locked);
}
async function operation(work: () => Promise<void>) {
  if (busy) return;
  busy = true; controls();
  try { await work(); } catch (e) { notify(String(e), true); }
  finally { busy = false; controls(); }
}
function setMode(next: 'send' | 'receive') {
  mode = next;
  $('send-form').hidden = next !== 'send'; $('receive-form').hidden = next !== 'receive';
  $('mode-send').setAttribute('aria-pressed', String(next === 'send'));
  $('mode-receive').setAttribute('aria-pressed', String(next === 'receive'));
  controls();
}
function budget() { return localStorage.getItem('hermod-budget') ?? '8'; }
function showBudget() { $('limit-summary').textContent = Number(budget()) === 0 ? 'Unlimited · upload may saturate' : `${budget()} Mbps payload cap`; }
$('bandwidth').setAttribute('value', budget()); showBudget();
$('theme').addEventListener('change', () => {
  const value = ($('theme') as HTMLSelectElement).value;
  localStorage.setItem('hermod-theme', value); document.documentElement.dataset.theme = value;
});
const savedTheme = localStorage.getItem('hermod-theme') ?? 'system';
document.documentElement.dataset.theme = savedTheme;
($('theme') as HTMLSelectElement).value = savedTheme;
document.querySelectorAll<HTMLElement>('[data-view]').forEach(el => el.addEventListener('click', () => navigate(el.dataset.view as View)));
$('mode-send').addEventListener('click', () => setMode('send'));
$('mode-receive').addEventListener('click', () => setMode('receive'));
$('edit-budget').addEventListener('click', () => navigate('settings'));
$('save-settings').addEventListener('click', () => {
  const field = $<Field>('bandwidth'); const n = Number(field.value);
  if (field.value.trim() === '' || !Number.isFinite(n) || n < 0 || n > 1000000 || (n > 0 && n < 0.001)) {
    field.error = true; field.errorText = 'Enter 0 or a value from 0.001 to 1000000.'; return;
  }
  field.error = false; localStorage.setItem('hermod-budget', String(n)); showBudget(); notify('Budget saved for the next transfer.');
});
$('choose-file').addEventListener('click', () => void operation(async () => {
  const source = await invoke<Selection | null>('choose_source');
  if (source) { selected = source; $('file-name').textContent = source.name; $('file-size').textContent = size(source.size); }
}));
$('start-transfer').addEventListener('click', () => void operation(async () => {
  const code = $<Field>('receive-code').value.trim();
  if (mode === 'receive' && !/^\d+-\S+-\S+-\S+$/.test(code)) {
    $<Field>('receive-code').error = true; $<Field>('receive-code').errorText = 'Enter the full code from your sender.'; return;
  }
  $<Field>('receive-code').error = false;
  await invoke('start_transfer', {sourceId: mode === 'send' ? selected?.id : null, code: mode === 'receive' ? code : null, limitMbps: budget()});
  running = true; previousDone = false;
  if (mode === 'send') { selected = null; $('file-name').textContent = 'Choose a file'; $('file-size').textContent = 'Browse files on this device'; }
  await poll();
}));
$('copy-code').addEventListener('click', () => void operation(async () => { await invoke('copy_code'); notify('Transfer code copied.'); }));
$('cancel-transfer').addEventListener('click', () => void operation(async () => { await invoke('cancel_transfer'); await poll(); }));
$('refresh-inbox').addEventListener('click', () => void refreshInbox());
function renderStatus(s: Status) {
  running = !s.done;
  $('status-label').textContent = s.error ? 'CONNECTION NEEDS ATTENTION' : s.cancelled ? 'CANCELLED' : s.done ? 'TRANSFER COMPLETE' : 'TRANSFER IN PROGRESS';
  $('status-dot').classList.toggle('failed', Boolean(s.error));
  $('status-title').textContent = s.phase;
  $('status-detail').textContent = s.error ?? (s.cancelled ? 'You can start another transfer when you are ready.' : s.done ? 'The file arrived and its SHA-256 digest matches.' : s.code ? 'Share this code with your receiver. Leave Hermod open while they connect.' : 'Connecting securely to your peer.');
  $('code-panel').hidden = !s.code || s.done;
  $('transfer-code').textContent = s.code ?? '';
  $('cancel-transfer').hidden = s.done;
  $('connection-art').hidden = true;
  $('progress-panel').hidden = !s.bytes && !s.total;
  const progress = $('progress') as HTMLElement & { value: number; indeterminate: boolean };
  progress.indeterminate = s.total === null && !s.done;
  progress.value = s.total ? Math.min(1, s.bytes/s.total) : s.done ? 1 : 0;
  $('progress-bytes').textContent = `${size(s.bytes)}${s.total === null ? '' : ` / ${size(s.total)}`}`;
  $('progress-percent').textContent = s.total ? `${Math.min(100, s.bytes/s.total*100).toFixed(0)}%` : '';
  $('speed').textContent = s.elapsed > 0 ? `${(s.bytes/s.elapsed/125000).toFixed(1)} Mbps` : '—';
  $('rtt').textContent = s.rtt_ms === null ? '—' : `${s.rtt_ms.toFixed(0)} ms`;
  $('digest-panel').hidden = !s.digest;
  $('digest').textContent = s.digest ?? '';
  if (s.done && !previousDone) { void refreshInbox(); if (s.error) notify(s.error, true); }
  previousDone = s.done;
  controls();
}
async function refreshInbox() {
  try {
    files = await invoke<Received[]>('list_received');
    $('inbox-count').textContent = `${files.length} verified ${files.length === 1 ? 'file' : 'files'}`;
    $('inbox-list').innerHTML = files.length ? files.map(f => `<article class="card inbox-item"><div class="inbox-file-icon">${icon('file')}</div><div class="inbox-info"><h2>${escape(f.name)}</h2><p>${size(f.size)} <span class="verified-label">✓ SHA-256 verified</span></p><details><summary>View digest</summary><code>${escape(f.sha256)}</code></details></div><div class="inbox-actions"><md-filled-button data-export="${escape(f.id)}">Save a copy</md-filled-button><md-text-button data-delete="${escape(f.id)}">Remove</md-text-button></div></article>`).join('') : `<div class="card empty-inbox">${icon('inbox')}<h2>A little space for what is next.</h2><p>Your completed, verified downloads will appear here.</p><md-outlined-button id="inbox-receive">Receive a file</md-outlined-button></div>`;
    $('inbox-receive')?.addEventListener('click', () => { setMode('receive'); navigate('transfer'); });
    document.querySelectorAll<HTMLElement>('[data-export]').forEach(el => el.addEventListener('click', () => void operation(async () => {
      if (await invoke<boolean>('export_received', {id: el.dataset.export})) notify('File saved. Your verified inbox copy is retained.');
    })));
    document.querySelectorAll<HTMLElement>('[data-delete]').forEach(el => el.addEventListener('click', () => {
      deleteId = el.dataset.delete!; ($('delete-dialog') as HTMLDialogElement).showModal();
    }));
    controls();
  } catch (e) { notify(`Cannot load inbox: ${String(e)}`, true); }
}
let deleteId = '';
$('delete-cancel').addEventListener('click', () => ($('delete-dialog') as HTMLDialogElement).close());
$('delete-confirm').addEventListener('click', () => void operation(async () => {
  ($('delete-dialog') as HTMLDialogElement).close();
  await invoke('delete_received', { id: deleteId }); await refreshInbox(); notify('Inbox copy removed.');
}));
let polling = false;
let pollFailed = false;
async function poll() {
  if (polling) return;
  polling = true;
  try {
    const s = await invoke<Status | null>('status');
    if (s && JSON.stringify(s) !== JSON.stringify(lastStatus)) { renderStatus(s); lastStatus = s; }
    pollFailed = false;
  } catch (e) { if (!pollFailed) notify(`Cannot read transfer status: ${String(e)}`, true); pollFailed = true; }
  finally { polling = false; }
}
async function boot() {
  controls();
  try {
    const info = await invoke<Bootstrap>('bootstrap');
    $('connection-badge').innerHTML = `<span class="dot ${info.ipv6.length ? '' : 'failed'}"></span>${info.ipv6.length ? 'IPv6 available' : 'IPv6 unavailable'}`;
    $('network-warning').hidden = Boolean(info.ipv6.length);
    $('server-address').textContent = info.server;
    $('ipv6-addresses').textContent = info.ipv6.join('\n') || 'No usable address detected';
    $('app-version').textContent = `Hermod ${info.version} · ${info.platform}`;
    await poll();
    setInterval(() => { if (!document.hidden || running) void poll(); }, 400);
    document.addEventListener('visibilitychange', () => { if (!document.hidden) { void poll(); if (view === 'inbox') void refreshInbox(); } });
  } catch (e) { notify(`Cannot connect to the native app: ${String(e)}`, true); }
}
void boot();
