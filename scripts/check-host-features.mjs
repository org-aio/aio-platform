import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';

// 只核对生产与构建依赖，测试客户端不应改变发布宿主的编译边界。
const profiles = [
  'components', 'server', 'sidecar', 'wasm', 'rhai', 'typescript', 'mcp', 'grpc',
  'remote-mcp', 'remote-grpc', 'remote', 'wasm,sidecar,rhai,typescript,mcp,grpc,remote',
];
const implementations = [
  { features: ['wasm'], packages: /^(wasmtime|cranelift)(-|$)/ },
  { features: ['rhai'], packages: /^rhai$/ },
  { features: ['typescript'], packages: /^(rquickjs($|-)|oxc_)/ },
  { features: ['mcp', 'remote-mcp', 'remote'], packages: /^rmcp$/ },
  { features: ['grpc', 'remote-grpc', 'remote'], packages: /^(tonic|prost)($|-)/ },
  { features: ['grpc'], packages: /^(tonic-prost-build|protoc-bin-vendored)($|-)/ },
  { features: ['remote-mcp', 'remote'], packages: /^process-wrap$/ },
  { features: ['mcp', 'remote-mcp', 'remote'], packages: /^sse-stream$/ },
];

for (const profile of profiles) {
  const tree = execFileSync('cargo', [
    'tree', '--locked', '-p', 'az-plugin-host', '--no-default-features',
    '--features', profile, '-e', 'normal,build', '--prefix', 'none',
  ], { encoding: 'utf8' });
  const dependencies = [...new Set(tree.split('\n').map(line => line.split(' ')[0]))];
  const selected = new Set(profile.split(','));
  if (profile === 'components') {
    assert.equal(dependencies.some(name => /^(sqlx($|-)|sqlparser$)/.test(name)), false,
      'components contracts must not select a database implementation');
  }
  for (const { features, packages } of implementations) {
    const enabled = features.some(feature => selected.has(feature));
    const matches = dependencies.filter(name => packages.test(name));
    assert.equal(matches.length > 0, enabled,
      `${profile}: ${features.join('/')} dependencies: ${matches.join(', ') || '(absent)'}`);
  }
  console.log(`${profile}: production dependencies verified`);
}

// 独立执行运行时必须能省略数据库驱动，默认 PostgreSQL 实现也不能带入 Wasmtime。
for (const profile of ['', 'wasm', 'postgres', 'wasm,postgres']) {
  const args = ['tree', '--locked', '-p', 'az-plugin-runtime', '--no-default-features',
    '-e', 'normal,build', '--prefix', 'none'];
  if (profile) {
    args.push('--features', profile);
  }
  const dependencies = execFileSync('cargo', args, { encoding: 'utf8' })
    .split('\n').map(line => line.split(' ')[0]);
  for (const packages of [/^sqlx($|-)/, /^sqlparser$/]) {
    assert.equal(dependencies.some(name => packages.test(name)), profile.includes('postgres'),
      `runtime ${profile || '(contracts)'}: unexpected database dependencies`);
  }
  assert.equal(dependencies.some(name => /^(wasmtime|cranelift)(-|$)/.test(name)), profile.includes('wasm'));
  console.log(`runtime ${profile || '(contracts)'}: production dependencies verified`);
}
