const {test}=require('node:test');
const assert=require('node:assert/strict');
const {versionFor,configuration,sameRelease}=require('./release.cjs');
test('开发版本保持高于基础版本，标签必须精确匹配',()=>{
  assert.deepEqual(versionFor('0.4.1','refs/heads/main','7','a'.repeat(40)),{version:'0.4.2-dev.7.gaaaaaaaaaaaa',tag:'next'});
  assert.deepEqual(versionFor('0.4.2','refs/tags/v0.4.2','8','a'.repeat(40)),{version:'0.4.2',tag:'latest'});
  for(const args of [['0.4.2','refs/tags/v0.4.1','7','a'.repeat(40)],['0.4.1','refs/pull/7/merge','7','a'.repeat(40)],['0.4.1','refs/heads/main','7','main']]) assert.throws(()=>versionFor(...args));
});
test('入口必须存在且安装参数不允许控制字符',()=>{
  const pkg={name:'tool',bin:{tool:'dist/cli.mjs'}};
  const config={id:'tool',title:'工具',command:'tool',platforms:['macos']};
  assert.deepEqual(configuration(pkg,{...config}).setup,[]);
  assert.throws(()=>configuration(pkg,{...config,command:'other'}));
  assert.throws(()=>configuration(pkg,{...config,setup:['setup\nother']}));
});
test('重复发布需匹配源码和包完整性',()=>{
  const release={package:'tool',version:'0.1.1-dev.1',source:{repository:'owner/tool',revision:'a'.repeat(40),reference:'refs/heads/main'}};
  const published={name:release.package,version:release.version,aio:{source:release.source},dist:{integrity:'sha512-test'}};
  assert(sameRelease(published,release,'sha512-test'));
  assert(!sameRelease(published,release,'sha512-other'));
  assert(!sameRelease({...published,aio:{source:{...release.source,reference:'refs/heads/other'}}},release));
  assert(!sameRelease({...published,aio:{source:{...release.source,repository:'other/tool'}}},release));
});
