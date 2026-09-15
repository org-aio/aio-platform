import type { Greeting } from './model.js';
export function greet(input: Greeting): string { return `你好，${input.name}！`; }
