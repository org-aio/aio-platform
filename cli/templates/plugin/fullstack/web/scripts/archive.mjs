import { readdir, readFile, readlink, writeFile } from 'node:fs/promises';
import { join, relative, resolve, dirname, sep } from 'node:path';
import { brotliCompressSync, constants } from 'node:zlib';

export async function archive(directory, entry) {
  const files = {};
  const links = {};
  const root = resolve(directory);
  let bytes = 0;
  async function walk(path) {
    for (const item of await readdir(path, { withFileTypes: true })) {
      const source = join(path, item.name);
      const name = relative(directory, source).split('\\').join('/');
      if (item.name.endsWith('.map')) continue;
      if (item.isSymbolicLink()) {
        const link = await readlink(source);
        const target = resolve(dirname(source), link);
        if (!target.startsWith(root + sep)) throw new Error(`服务端依赖逃逸: ${name}`);
        links[name] = relative(dirname(resolve(source)), target).split(sep).join('/');
        continue;
      }
      if (item.isDirectory()) await walk(source);
      else {
        const data = await readFile(source);
        bytes += data.length;
        files[name] = brotliCompressSync(data, { params: { [constants.BROTLI_PARAM_QUALITY]: 4 } }).toString('base64');
      }
    }
  }
  await walk(directory);
  const bootstrap = await readFile(new URL('./runtime.cjs', import.meta.url), 'utf8');
  await writeFile('dist/server.cjs', bootstrap.replace('__ARCHIVE__', () => JSON.stringify({ entry, files, links })));
  console.log(`服务端展开大小: ${(bytes / 1024 / 1024).toFixed(1)} MiB`);
}
