import { cp, mkdir, writeFile, rm } from 'node:fs/promises';
import { archive } from './archive.mjs';
await rm('dist/frontend', { recursive: true, force: true });
await mkdir('dist/frontend', { recursive: true });
await cp('frontend/.next/static', 'dist/frontend/_next/static', { recursive: true });
await cp('frontend/.next/server/app/index.html', 'dist/frontend/index.html');
await writeFile('frontend/.next/standalone/frontend/package.json', JSON.stringify({ type: 'commonjs' }));
await archive('frontend/.next/standalone', 'frontend/server.js');
