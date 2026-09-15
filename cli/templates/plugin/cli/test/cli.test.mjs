import {test} from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
const cli=(...args)=>execFileSync(process.execPath,['dist/cli.mjs',...args],{encoding:'utf8'}).trim();
test('显示打包版本和用户输入',()=>{
  assert.equal(cli('--version'),JSON.parse(readFileSync('package.json','utf8')).version);
  assert.equal(cli('--name','AIO'), '你好，AIO！');
});
