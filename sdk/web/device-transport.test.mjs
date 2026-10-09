import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {webcrypto} from 'node:crypto';

const publicOrigin='https://public.example';
const lanOrigin='https://lan.example:3443';
const source=readFileSync(new URL('./guest.js',import.meta.url),'utf8');
function guest(fetch) {
  const window={fetch,parent:{postMessage(){}},addEventListener(){}};
  vm.runInContext(source,vm.createContext({window,fetch,AbortSignal,crypto:webcrypto,URL,Uint8Array,TextEncoder,TextDecoder,setTimeout,clearTimeout,
    document:{currentScript:{dataset:{root:publicOrigin+'/api/runtime/components/assets/mount/',lanOrigins:JSON.stringify([lanOrigin])}}}}));
  return window.aioPlugin;
}

test('设备视图经无凭据探测后使用局域网，并保留票据作用域',async()=>{
  const calls=[];
  const api=guest(async(url,options)=>{
    calls.push({url:String(url),options});
    assert.equal(options.credentials,'omit');
    assert.equal(options.redirect,'error');
    return {ok:true,json:async()=>({data:String(url).endsWith('/transport')?{public_origin:publicOrigin,lan_origins:[lanOrigin]}:{id:'view'}})};
  });
  await Promise.all([api.deviceView({operation:'list'}),api.deviceView({operation:'open',device:'fixture'})]);
  assert.equal(calls.filter(call=>call.url.endsWith('/transport')).length,1);
  assert(calls.slice(1).every(call=>call.url===lanOrigin+'/api/runtime/components/assets/mount/__device_view'));
  assert.equal(calls[0].options.headers,undefined);
});

test('不可达、浏览器权限拒绝或其他宿主均回退公网',async()=>{
  for(const mode of ['unreachable','permission','identity']) {
    const calls=[];
    const api=guest(async(url)=>{
      calls.push(String(url));
      if(String(url).endsWith('/transport')) {
        if(mode!=='identity'){throw new TypeError(mode);}
        return {ok:true,json:async()=>({data:{public_origin:'https://different.example',lan_origins:[lanOrigin]}})};
      }
      return {ok:true,json:async()=>({data:[]})};
    });
    await api.deviceView({operation:'list'});
    assert.equal(calls.at(-1),publicOrigin+'/api/runtime/components/assets/mount/__device_view');
  }
});

test('已提交的操作失败后不会自动重放到公网',async()=>{
  let operations=0;
  const calls=[];
  const api=guest(async(url)=>{
    calls.push(String(url));
    if(String(url).endsWith('/transport')){return {ok:true,json:async()=>({data:{public_origin:publicOrigin,lan_origins:[lanOrigin]}})};}
    operations++;
    if(String(url).startsWith(lanOrigin)){throw new TypeError('response lost');}
    return {ok:true,json:async()=>({data:[]})};
  });
  await assert.rejects(api.deviceView({operation:'open',device:'fixture'}),/response lost/);
  assert.equal(operations,1);
  await api.deviceView({operation:'list'});
  assert.equal(operations,2);
  assert.equal(calls.at(-1),publicOrigin+'/api/runtime/components/assets/mount/__device_view');
});
