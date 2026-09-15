import { cp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { archive } from './archive.mjs';
await rm('dist/frontend', { recursive: true, force: true });
await mkdir('dist/frontend', { recursive: true });
await cp('.output/public', 'dist/frontend', { recursive: true });
const entry = await readFile('dist/frontend/index.html', 'utf8');
await writeFile('dist/frontend/index.html', entry.replaceAll('"/_nuxt/', '"./_nuxt/'));
await archive('.output/server', 'index.mjs');
