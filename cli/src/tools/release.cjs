const fs = require('node:fs');
const path = require('node:path');
const {execFileSync} = require('node:child_process');
const {createHash} = require('node:crypto');
const registry = 'https://registry.npmjs.org';
const market = 'https://aio.addzero.site';
const stateFile = '.aio/cli-release.json';
const read = file => JSON.parse(fs.readFileSync(file, 'utf8'));
const write = (file, value) => { fs.mkdirSync(path.dirname(file), {recursive:true}); fs.writeFileSync(file, JSON.stringify(value, null, 2)+'\n'); };
const fail = message => { throw new Error(message); };

function versionFor(base, reference, run, sha) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(base)) fail('package.json version 必须是正式 SemVer');
  if (!/^[a-f0-9]{40}$/.test(sha)) fail('需要完整的源码提交 SHA');
  if (reference.startsWith('refs/tags/')) {
    if (reference !== `refs/tags/v${base}`) fail('发布标签必须与 package.json version 一致');
    return {version:base, tag:'latest'};
  }
  if (!reference.startsWith('refs/heads/') || !/^[1-9]\d*$/.test(run)) fail('需要默认分支和 GitHub run number');
  const [major,minor,patch]=base.split('.').map(Number);
  return {version:`${major}.${minor}.${patch+1}-dev.${run}.g${sha.slice(0,12)}`,tag:'next'};
}

function configuration(pkg, config) {
  if (!/^[a-z0-9][a-z0-9-]{0,79}$/.test(config.id||'')) fail('aio-cli.json id 无效');
  if (typeof config.title!=='string' || !config.title.trim() || config.title.length>200) fail('CLI 标题无效');
  if (!/^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$/.test(config.command||'')) fail('CLI 命令名称无效');
  const bin=typeof pkg.bin==='string'?{[pkg.name]:pkg.bin}:pkg.bin;
  if (!bin || typeof bin[config.command]!=='string') fail('CLI 命令必须在 package.json bin 中声明');
  if (!Array.isArray(config.platforms) || !config.platforms.length || config.platforms.some(p=>!['macos','linux','windows'].includes(p))) fail('CLI 平台无效');
  for (const key of ['setup','uninstall']) {
    config[key]??=[];
    if(!Array.isArray(config[key])||config[key].length>64||config[key].some(v=>typeof v!=='string'||/[\x00-\x1f\x7f]/.test(v)||v.length>4096)) fail(`${key} 必须是参数数组`);
  }
  return config;
}

function prepare(env=process.env) {
  const pkg=read('package.json');
  const repository=env.GITHUB_REPOSITORY||'';
  if(!/^[a-zA-Z0-9_.-]+\/[a-zA-Z0-9_.-]+$/.test(repository)) fail('此命令在 GitHub Actions 中运行，需要 GITHUB_REPOSITORY');
  const source={repository,revision:env.GITHUB_SHA,reference:env.GITHUB_REF};
  const release=versionFor(pkg.version,source.reference,env.GITHUB_RUN_NUMBER,source.revision);
  const cli=configuration(pkg,read('aio-cli.json'));
  const expected=`https://github.com/${repository}`;
  const declared=typeof pkg.repository==='string'?pkg.repository:pkg.repository?.url;
  if(declared && declared.replace(/^git\+/,'').replace(/\.git$/,'')!==expected) fail('package.json repository 必须指向当前 GitHub 仓库');
  pkg.version=release.version;
  pkg.repository={type:'git',url:`git+${expected}.git`};
  pkg.aio={cli,source};
  pkg.publishConfig={...pkg.publishConfig,registry,access:'public'};
  write('package.json',pkg);
  write(stateFile,{...release,package:pkg.name,command:cli.command,source});
  console.log(`准备发布 ${pkg.name}@${release.version} (${release.tag})`);
  return release;
}

function npm(args, capture=false) {
  const executable=process.env.npm_execpath;
  const options={encoding:'utf8',stdio:capture?['ignore','pipe','inherit']:'inherit',env:{...process.env,npm_config_registry:registry}};
  if (executable && /npm-cli\.js$/.test(executable) && fs.existsSync(executable)) return execFileSync(process.execPath,[executable,...args],options);
  if(process.platform==='win32') fail('请通过 npx @zjarlin/aio 调用发布工具');
  return execFileSync('npm',args,options);
}

async function setup() {
  const pkg=read('package.json');
  configuration(pkg,read('aio-cli.json'));
  const remote=execFileSync('git',['remote','get-url','origin'],{encoding:'utf8'}).trim();
  const repository=remote.replace(/^git@github\.com:/,'').replace(/^https:\/\/github\.com\//,'').replace(/\.git$/,'');
  if(!/^[a-zA-Z0-9_.-]+\/[a-zA-Z0-9_.-]+$/.test(repository)) fail('请先设置当前项目的 GitHub origin');
  if(!fs.existsSync('.github/workflows/aio-cli.yml')) fail('请先用 aio plugin init --kind cli 初始化或接入当前项目');
  pkg.repository={type:'git',url:`git+https://github.com/${repository}.git`};
  write('package.json',pkg);
  const response=await fetch(`${registry}/${encodeURIComponent(pkg.name)}/latest`,{signal:AbortSignal.timeout(30000)});
  if(response.status===404) {
    npm(['test']);
    npm(['publish','--registry',registry,'--access','public']);
  } else if(!response.ok) fail(`检查 npm 包失败：HTTP ${response.status}`);
  console.log(`配置 ${pkg.name} 的自动发布身份：${repository}/aio-cli.yml`);
  npm(['exec','--yes','--package=npm@11','--','npm','trust','github',pkg.name,'--repo',repository,'--file','aio-cli.yml','--allow-publish','--yes']);
  console.log('自动发布已配置；提交源码后将依次发布 npm 并更新 AIO 插件市场。');
}

async function metadata(name, version) {
  const response=await fetch(`${registry}/${encodeURIComponent(name)}/${encodeURIComponent(version)}`,{signal:AbortSignal.timeout(30000)});
  if(response.status===404) return null;
  if(!response.ok) fail(`读取 npm 版本失败：HTTP ${response.status}`);
  return response.json();
}

function sameRelease(value, release, integrity) {
  return value?.name===release.package && value.version===release.version && value.aio?.source?.repository===release.source.repository && value.aio?.source?.revision===release.source.revision && value.aio?.source?.reference===release.source.reference && (!integrity || value.dist?.integrity===integrity);
}

async function publish(runNpm = npm) {
  const release=read(stateFile);
  fs.mkdirSync('.aio/npm',{recursive:true});
  const packages=JSON.parse(runNpm(['pack','--ignore-scripts','--json','--pack-destination','.aio/npm'],true));
  if(packages.length!==1) fail('必须生成一个 npm 包');
  const archive=path.resolve('.aio/npm',packages[0].filename);
  const integrity='sha512-'+createHash('sha512').update(fs.readFileSync(archive)).digest('base64');
  // 使用打包后的命令做版本验证，不执行 setup 或其他本机修改。
  const reported=runNpm(['exec','--yes',`--package=${archive}`,'--',release.command,'--version'],true).trim();
  if(!reported.split(/\s+/).includes(release.version)) fail(`打包后 CLI --version 必须返回 ${release.version}`);
  const existing=await metadata(release.package,release.version);
  if(existing) {
    if(!sameRelease(existing,release,integrity)) fail('npm 已存在不同内容的同名版本，拒绝覆盖');
    console.log('npm 已有相同发布，继续同步市场');
  } else {
    // npm OIDC 根据仓库可见性自动生成来源证明，私有仓库不能强制开启。
    runNpm(['publish',archive,'--registry',registry,'--access','public','--tag',release.tag,'--ignore-scripts']);
  }
  write(stateFile,{...release,integrity});
}

async function identity() {
  const url=new URL(process.env.ACTIONS_ID_TOKEN_REQUEST_URL||fail('需要 GitHub Actions id-token: write'));
  if(url.protocol!=='https:') fail('OIDC 请求地址必须是 HTTPS');
  url.searchParams.set('audience',market);
  const response=await fetch(url,{headers:{authorization:`Bearer ${process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN||fail('缺少 OIDC 请求凭据')}`},signal:AbortSignal.timeout(30000)});
  if(!response.ok) fail(`获取发布身份失败：HTTP ${response.status}`);
  const value=(await response.json()).value;
  if(typeof value!=='string') fail('OIDC 未返回发布身份');
  return value;
}

async function sync(wait = ms => new Promise(resolve => setTimeout(resolve, ms))) {
  const release=read(stateFile);
  for(let attempt=0;attempt<60;attempt++) {
    const value=await metadata(release.package,release.version);
    if(value && !sameRelease(value,release,release.integrity)) fail('npm 版本来源或完整性与本次构建不一致');
    if(value) {
      const response=await fetch(`${market}/api/runtime/tools/publish`,{method:'POST',headers:{'content-type':'application/json',authorization:`Bearer ${await identity()}`},body:JSON.stringify({package:release.package,version:release.version}),signal:AbortSignal.timeout(60000)});
      if(response.ok) {console.log(`已发布 npm 并更新 AIO 市场：${release.package}@${release.version}`);return;}
      if(response.status<500 && response.status!==404) fail(`市场拒绝发布：HTTP ${response.status} ${(await response.text()).slice(0,1200)}`);
    }
    if(attempt===0 || (attempt+1)%6===0) console.log(`等待 npm 与市场更新：第 ${attempt+1} 次检查`);
    if(attempt<59) await wait(10000);
  }
  fail('npm 或市场尚未就绪；重新运行相同工作流可继续同步，无需更改版本');
}

module.exports={versionFor,configuration,sameRelease,prepare,publish,sync};
if(!module.parent) {
  const mode=process.argv[1];
  Promise.resolve().then(()=>{
    if(mode==='setup') return setup();
    if(mode==='prepare') return prepare();
    if(mode==='publish') return publish();
    if(mode==='sync') return sync();
    fail('用法：aio tool release setup|prepare|publish|sync');
  }).catch(error=>{console.error(error.message);process.exitCode=1;});
}
