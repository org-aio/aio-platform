import { build } from 'esbuild';
import { mkdir, copyFile, access, writeFile } from 'node:fs/promises';
const target = process.argv[2];
if (target === 'frontend') {
  await mkdir('dist/frontend', { recursive: true });
  await build({ entryPoints: ['frontend/main.ts'], bundle: true, format: 'esm', outfile: 'dist/frontend/main.js', sourcemap: 'inline', sourcesContent: true });
  await copyFile('frontend/index.html', 'dist/frontend/index.html');
  const cached = '.aio/dev/cache/shared.css';
  try { await access(cached); } catch {
    const response = await fetch('https://raw.githubusercontent.com/zjarlin/dioxus-admin-workbench/22ee3cb9324f90e1833080d01663b92c28929c75/crates/ui/components/src/plugin_surface/style.css', { signal: AbortSignal.timeout(30000) });
    if (!response.ok) throw new Error(`Shared stylesheet: ${response.status}`);
    await mkdir('.aio/dev/cache', { recursive: true });
    await writeFile(cached, await response.text());
  }
  await copyFile(cached, 'dist/frontend/style.css');
} else if (target === 'backend') {
  await build({ entryPoints: ['backend/server.ts'], bundle: true, platform: 'node', format: 'esm', outfile: 'dist/server.js', sourcemap: 'inline', sourcesContent: true });
} else throw new Error('Unknown development task');
