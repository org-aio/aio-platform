#!/usr/bin/env node
import { parseArgs } from 'node:util';
import { greet } from './greeting/index.js';

declare const PACKAGE_VERSION: string;
const {values} = parseArgs({options:{version:{type:'boolean'},help:{type:'boolean'},name:{type:'string',default:'世界'}}});
if(values.version) console.log(PACKAGE_VERSION);
else if(values.help) console.log('__NAME__ [--name 名称] [--version]');
else console.log(greet({name:values.name}));
