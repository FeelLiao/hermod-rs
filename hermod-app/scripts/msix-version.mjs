import { fileURLToPath } from 'node:url';

// 商店包首段不能为 0，末段留给商店；固定偏移可避免未来 1.x 与 0.x 冲突。
export function msixVersion(version) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) {
    throw new Error('MSIX requires a stable major.minor.patch app version');
  }
  const [major, minor, patch] = version.split('.').map(Number);
  const parts = [major + 1, minor, patch, 0];
  if (parts.some(part => !Number.isSafeInteger(part) || part < 0 || part > 65535)) {
    throw new Error('MSIX version components must fit 0..65535 after the major offset');
  }
  return parts.join('.');
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  console.log(msixVersion(process.argv[2] ?? ''));
}
