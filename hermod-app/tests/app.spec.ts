import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    let status: Record<string, unknown> | null = null;
    const calls: {command: string; args: any}[] = [];
    const files = [{id: 'a'.repeat(32), name: '地球 3.0.zip', size: 10485760, sha256: 'f'.repeat(64)}];
    (window as any).calls = calls;
    (window as any).__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: any) => {
        calls.push({command, args});
        switch (command) {
          case 'bootstrap': return {version: '0.2.1', server:'wss://relay.example:4376', ipv6:['2001:db8::1'], platform:'test'};
          case 'status': return status;
          case 'choose_source': return {id:'source', name:'地球 3.0.zip', size:10485760};
          case 'start_transfer': status = {phase: 'Waiting for the receiver', code: args.sourceId ? '1234-word-word-word' : null, bytes:0,total:10485760,elapsed:0,done:false,cancelled:false,error:null,digest:null,rtt_ms:null}; return;
          case 'cancel_transfer': status = {...status,done:true,cancelled:true,phase:'Transfer cancelled'}; return;
          case 'list_received': return files;
          case 'export_received': return true;
          case 'delete_received': files.length = 0; return;
          case 'copy_code': return;
          default: throw new Error(`Unexpected command: ${command}`);
        }
      },
    };
  });
});

test('send, code, cancellation and narrow layout', async ({ page }, info) => {
  await page.goto('/');
  await expect(page.getByText('IPv6 available', {exact:true})).toBeVisible();
  await expect(page.locator('#start-transfer')).toHaveAttribute('disabled', '');
  await page.getByRole('button', {name:'Choose file', exact:true}).click();
  await expect(page.locator('#file-name')).toHaveText('地球 3.0.zip');
  await page.locator('#start-transfer').click();
  await expect(page.locator('#transfer-code')).toHaveText('1234-word-word-word');
  await page.locator('#copy-code').click();
  await expect(page.locator('#snackbar')).toContainText('copied');
  await page.locator('#cancel-transfer').click();
  await expect(page.locator('#status-title')).toHaveText('Transfer cancelled');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path:`test-results/${info.project.name}-transfer.png`, fullPage:true });
});

test('receiver validates codes and uses saved budget', async ({ page }) => {
  await page.goto('/');
  await page.locator('[data-view=settings]').click();
  await page.locator('#bandwidth').locator('input').fill('6.5');
  await page.locator('#save-settings').click();
  await page.locator('[data-view=transfer]').click();
  await page.locator('#mode-receive').click();
  await page.locator('#receive-code').locator('input').fill('bad');
  await page.locator('#start-transfer').click();
  expect(await page.locator('#receive-code').evaluate((e: any) => e.error)).toBe(true);
  await page.locator('#receive-code').locator('input').fill('1234-word-word-word');
  await page.locator('#start-transfer').click();
  await expect(page.locator('#cancel-transfer')).toBeVisible();
  expect(await page.evaluate(() => (window as any).calls.find((c:any) => c.command === 'start_transfer').args)).toEqual({sourceId:null,code:'1234-word-word-word',limitMbps:'6.5'});
});

test('verified inbox exports by ID, confirms removal and renders dark theme', async ({ page }, info) => {
  await page.goto('/');
  await page.locator('[data-view=inbox]').click();
  await expect(page.getByRole('heading', {name:'地球 3.0.zip'})).toBeVisible();
  await page.locator('[data-export]').click();
  await expect(page.locator('#snackbar')).toContainText('File saved');
  expect(await page.evaluate(() => (window as any).calls.find((c:any) => c.command === 'export_received').args)).toEqual({id:'a'.repeat(32)});
  await page.locator('[data-delete]').click();
  await expect(page.locator('#delete-dialog')).toBeVisible();
  await page.locator('#delete-cancel').click();
  await expect(page.locator('[data-delete]')).toBeVisible();
  await page.locator('[data-view=settings]').click();
  await page.locator('#theme').selectOption('dark');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({path:`test-results/${info.project.name}-settings-dark.png`, fullPage:true});
  await page.locator('[data-view=inbox]').click();
  await page.locator('[data-delete]').click();
  await page.locator('#delete-confirm').click();
  await expect(page.locator('#inbox-count')).toHaveText('0 verified files');
});
