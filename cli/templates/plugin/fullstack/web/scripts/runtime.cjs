const { mkdtempSync, mkdirSync, writeFileSync, rmSync, symlinkSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join, dirname, resolve, sep } = require('node:path');
const { brotliDecompressSync } = require('node:zlib');
const { pathToFileURL } = require('node:url');
const payload = __ARCHIVE__;
const directory = mkdtempSync(join(tmpdir(), 'aio-service-'));
process.on('exit', () => rmSync(directory, { recursive: true, force: true }));
for (const [name, data] of Object.entries(payload.files)) {
  const target = resolve(directory, name);
  if (!target.startsWith(directory + sep)) throw new Error('服务端归档路径无效');
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, brotliDecompressSync(Buffer.from(data, 'base64')));
}
for (const [name, link] of Object.entries(payload.links)) {
  const target = resolve(directory, name);
  if (!target.startsWith(directory + sep) || !resolve(dirname(target), link).startsWith(directory + sep)) {
    throw new Error('服务端依赖路径无效');
  }
  mkdirSync(dirname(target), { recursive: true });
  symlinkSync(link, target);
}
process.env.NODE_ENV = 'production';
process.env.PORT = process.env.AIO_PLUGIN_PORT || process.env.PORT || '8080';
process.env.HOSTNAME = '0.0.0.0';
process.env.HOST = '0.0.0.0';
process.chdir(directory);
import(pathToFileURL(join(directory, payload.entry)).href).catch(error => {
  console.error(error);
  process.exit(1);
});
