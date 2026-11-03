// macOS icons need transparent space around their visible rounded square.
// Keep app-icon.svg as the shared artwork; only regenerate the macOS ICNS.
// Usage: node scripts/generate-macos-icon.mjs [optional-preview.png]
import { readFileSync, writeFileSync, mkdtempSync, copyFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const source = readFileSync(join(root, 'app-icon.svg'), 'utf8');
const match = source.match(/^\s*<svg\b[^>]*>([\s\S]*)<\/svg>\s*$/);
if (!match || !source.includes('viewBox="0 0 256 256"')) {
  throw new Error('Expected the canonical 256 × 256 SVG artwork.');
}
const temporary = mkdtempSync(join(tmpdir(), 'twinlink-macos-icon-'));
try {
  const svg = join(temporary, 'macos.svg');
  const output = join(temporary, 'icons');
  // 24 px inset on each side: 208 / 256 = 81.25% visible width.
  writeFileSync(svg, `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"><g transform="translate(24 24) scale(0.8125)">${match[1]}</g></svg>`);
  const result = spawnSync('pnpm', ['tauri', 'icon', svg, '--output', output], {
    cwd: root, encoding: 'utf8',
  });
  if (result.status !== 0) throw new Error(result.error?.message || result.stderr || result.stdout);
  copyFileSync(join(output, 'icon.icns'), join(root, 'src-tauri/icons/icon.icns'));
  if (process.argv[2]) copyFileSync(join(output, '128x128@2x.png'), resolve(process.argv[2]));
  console.log('Updated macOS icon.icns: centered artwork, 81.25% visible width.');
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
