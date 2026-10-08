import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import {webcrypto} from 'node:crypto';
import {mountBridge} from './host.mjs';

function host() {
  const listeners = new Map();
  const previousWindow = globalThis.window;
  const previousDocument = globalThis.document;
  globalThis.window = {addEventListener:(kind, listener)=>{
    const group=listeners.get(kind)??new Set(); group.add(listener); listeners.set(kind,group);
  },removeEventListener:(kind, listener)=>listeners.get(kind)?.delete(listener)};
  globalThis.document = {hidden:false};
  const received=[];
  const source={postMessage:message=>received.push(message)};
  let lease='lease-1';
  const frame={contentWindow:source,isConnected:true,inert:false,clientWidth:400,clientHeight:300,
    checkVisibility:()=>true,getBoundingClientRect:()=>({left:10,top:20,right:410,bottom:320,width:400,height:300}),
    addEventListener:(kind,listener)=>listeners.set('frame-'+kind,new Set([listener])),
    removeEventListener:(kind,listener)=>listeners.get('frame-'+kind)?.delete(listener)};
  const dispose=mountBridge(frame,()=>assert.fail('file drops must not invoke the service'),{fileDrop:()=>lease});
  const emit=async(kind,event)=>{for(const listener of listeners.get(kind)??[]){await listener(event);}};
  const message=(kind,fields={},sender=source,origin='null')=>emit('message',{source:sender,origin,data:{protocol:'aio:plugin@2',kind,id:'subscription',...fields}});
  const arm=async()=>{await message('file-drop-subscribe',{enabled:true});await message('file-drag');};
  const drop=(items,trusted=true)=>{
    const event={isTrusted:trusted,clientX:110,clientY:120,preventDefault(){this.prevented=true;},stopImmediatePropagation(){this.stopped=true;},
      dataTransfer:{files:items.map(item=>item.file),items:items.map(item=>({kind:'file',getAsFile:()=>item.file,webkitGetAsEntry:()=>item.entry})),getData:()=>''}};
    return {event,run:()=>emit('drop',event)};
  };
  return {frame,received,message,arm,drop,emit,setLease:value=>{lease=value;},dispose:()=>{
    dispose();globalThis.window=previousWindow;globalThis.document=previousDocument;
  }};
}

test('host only arms the owned opaque active frame and ignores an untrusted drop',async()=>{
  const h=host();
  try{
    await h.message('file-drop-subscribe',{enabled:true},{},'null');await h.message('file-drag');assert.equal(h.frame.inert,false);
    await h.message('file-drop-subscribe',{enabled:true},h.frame.contentWindow,'https://outside.invalid');await h.message('file-drag');assert.equal(h.frame.inert,false);
    await h.message('file-drop-subscribe',{enabled:true});h.setLease(false);await h.message('file-drag');assert.equal(h.frame.inert,false);
    h.setLease('lease-2');await h.message('file-drag');assert.equal(h.frame.inert,true);
    const untrusted=h.drop([{file:new File(['private'],'note.txt')}],false);await untrusted.run();assert.equal(h.received.length,0);assert.equal(h.frame.inert,false);
    await h.arm();const actual=h.drop([{file:new File([Uint8Array.of(0,128,255)],'note.bin')}]);await actual.run();
    assert.equal(actual.event.prevented,true);assert.equal(actual.event.stopped,true);assert.deepEqual(h.received[0].point,{x:100,y:100});
    assert.deepEqual(new Uint8Array(await h.received[0].roots[0].file.arrayBuffer()),Uint8Array.of(0,128,255));
  }finally{h.dispose();}
});

test('a replaced lease, subscription or unloaded frame discards an in-progress directory read',async()=>{
  for(const cancel of [h=>h.setLease('new-lease'),h=>h.message('file-drop-subscribe',{enabled:false}),h=>h.emit('frame-load',{})]){
    const h=host();let finish;
    try{
      await h.arm();const entry={name:'folder',isDirectory:true,createReader:()=>({readEntries:resolve=>{finish=resolve;}})};
      const pending=h.drop([{file:new File([],'folder'),entry}]).run();assert.equal(typeof finish,'function');
      await cancel(h);finish([]);await pending;assert.equal(h.received.length,0);assert.equal(h.frame.inert,false);
    }finally{h.dispose();}
  }
});

test('out-of-frame drops stay with their original target and host reader errors reach the current guest',async()=>{
  const h=host();
  try{
    await h.arm();const outside=h.drop([{file:new File(['bytes'],'note.txt')}]);outside.event.clientX=500;await outside.run();
    assert.equal(outside.event.prevented,undefined);assert.equal(h.received.length,0);
    await h.arm();const entry={name:'folder',isDirectory:true,createReader:()=>({readEntries:(_resolve,reject)=>reject(new Error('directory unavailable'))})};
    await h.drop([{file:new File([],'folder'),entry}]).run();assert.equal(h.received[0].error,'directory unavailable');assert.equal(h.received[0].roots,undefined);
  }finally{h.dispose();}
});

test('guest file drop subscriptions bind deliveries to the parent and discard stale registrations',()=>{
  let receive;const sent=[];const handled=[];
  const parent={postMessage:message=>sent.push(message)};
  const window={parent,addEventListener:(_kind,listener)=>{receive=listener;}};
  const context=vm.createContext({window,document:{currentScript:{dataset:{}}},crypto:webcrypto,Uint8Array,TextEncoder,TextDecoder,URL,setTimeout,clearTimeout});
  vm.runInContext(readFileSync(new URL('./guest.js',import.meta.url),'utf8'),context);
  const disposeFirst=window.aioPlugin.onFileDrop(message=>handled.push(message));const first=sent[0].id;
  window.aioPlugin.fileDrag();assert.equal(sent[1].kind,'file-drag');assert.equal(sent[1].id,first);
  receive({source:{},data:{protocol:'aio:plugin@2',kind:'file-drop',id:first,roots:[]}});assert.equal(handled.length,0);
  const disposeSecond=window.aioPlugin.onFileDrop(message=>handled.push(message));const second=sent[2].id;disposeFirst();
  receive({source:parent,data:{protocol:'aio:plugin@2',kind:'file-drop',id:first,roots:[]}});assert.equal(handled.length,0);
  receive({source:parent,data:{protocol:'aio:plugin@2',kind:'file-drop',id:second,roots:[]}});assert.equal(handled.length,1);
  disposeSecond();window.aioPlugin.fileDrag();assert.equal(sent.at(-1).enabled,false);
  receive({source:parent,data:{protocol:'aio:plugin@2',kind:'file-drop',id:second,roots:[]}});assert.equal(handled.length,1);
  receive({source:parent,data:{protocol:'aio:plugin@2',kind:'file-drop',roots:[]}});assert.equal(handled.length,1);
});
