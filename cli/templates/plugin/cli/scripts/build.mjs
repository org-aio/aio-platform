import {build} from 'esbuild';
import {readFile,chmod} from 'node:fs/promises';
const {version}=JSON.parse(await readFile(new URL('../package.json',import.meta.url),'utf8'));
await build({entryPoints:['src/cli.ts'],bundle:true,platform:'node',target:'node22',format:'esm',outfile:'dist/cli.mjs',define:{PACKAGE_VERSION:JSON.stringify(version)}});
await chmod('dist/cli.mjs',0o755);
