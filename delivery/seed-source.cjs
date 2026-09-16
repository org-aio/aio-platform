const { createHash } = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

const git = process.env.AIO_GIT || '/opt/aio-delivery/git/bin/git';
const [plugin, image, dependency, revision, bundle] = process.argv.slice(2);
if (process.getuid() !== 0 || !/^sha256:[a-f0-9]{64}$/.test(image || '') || !/^[a-f0-9]{40}$/.test(revision || '') || !bundle) {
  throw new Error('由 root 执行: node seed-source.cjs <插件 HTTPS Git> <构建镜像 digest> <依赖 HTTPS Git> <完整 SHA> <Git bundle>');
}
for (const value of [plugin, dependency]) {
  const url = new URL(value);
  if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash) throw new Error('Git 地址无效');
}
const hash = value => createHash('sha256').update(value).digest('hex');
const cache = path.join('/opt/aio-delivery/cache', hash(`${plugin}:${image}`));
const source = path.join(cache, '.cache/aio/sources', hash(dependency));
fs.mkdirSync(source, { recursive: true });
execFileSync(git, ['init', '-q', source]);
execFileSync(git, ['-C', source, 'bundle', 'unbundle', path.resolve(bundle)], { stdio: 'inherit' });
execFileSync(git, ['-C', source, 'cat-file', '-e', `${revision}^{commit}`]);
execFileSync('chown', ['-R', '65534:65534', cache]);
console.log(`已预置锁定依赖 ${dependency}@${revision}，仅供 ${plugin} 的隔离构建缓存使用。`);
