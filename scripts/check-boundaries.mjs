import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

const platform = fileURLToPath(new URL('../', import.meta.url));
const product = process.argv[2];
const business = /^(aio-plugin-|az-(studio|biz-|aio-app$))/;
const failures = [];
function check(root, label) {
  const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], { cwd: root, encoding: 'utf8' }));
  const members = new Set(metadata.workspace_members);
  for (const pkg of metadata.packages.filter(pkg => members.has(pkg.id))) {
    if (business.test(pkg.name)) failures.push(`${label}: business package ${pkg.name}`);
    for (const dependency of pkg.dependencies) {
      if (business.test(dependency.name)) failures.push(`${label}: ${pkg.name} depends on ${dependency.name}`);
      if (pkg.name === 'az-plugin-enhancement' && dependency.kind !== 'dev'
        && /^(az-plugin-(host|runtime|core)|sqlx|axum|reqwest|dioxus|tokio)$/.test(dependency.name)) {
        failures.push(`${label}: script evaluator must not own host infrastructure: ${dependency.name}`);
      }
    }
  }
}
check(platform, 'platform');
for (const name of ['app', 'lib/biz', 'generated/apps']) {
  if (existsSync(resolve(platform, name))) failures.push(`platform: application directory ${name}`);
}
if (product) check(resolve(product), 'product');
if (failures.length) {
  console.error(failures.join('\n'));
  process.exitCode = 1;
} else console.log(`Repository boundaries passed${product ? ' (platform + product)' : ' (platform only)'}`);
