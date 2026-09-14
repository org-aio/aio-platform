"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const path = require("node:path");
const os = require("node:os");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const test = require("node:test");
const { platformPackage, executableName } = require("../lib/platform.cjs");

test("npm 入口将停止信号传给宿主 CLI 并等待清理", { skip: process.platform === "win32", timeout: 10000 }, async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "aio-launcher-"));
  let child;
  try {
    await fs.cp(path.resolve(__dirname, "../bin"), path.join(root, "bin"), { recursive: true });
    await fs.cp(path.resolve(__dirname, "../lib"), path.join(root, "lib"), { recursive: true });
    const binary = path.join(root, "node_modules", platformPackage(), "bin", executableName());
    await fs.mkdir(path.dirname(binary), { recursive: true });
    await fs.writeFile(binary, `#!${process.execPath}\nprocess.on('SIGTERM',()=>setTimeout(()=>{console.log('cleaned');process.exit(0)},100));console.log('ready');setInterval(()=>{},1000);\n`, { mode: 0o755 });
    child = spawn(process.execPath, [path.join(root, "bin/aio.cjs")], { stdio: ["ignore", "pipe", "pipe"] });
    const exit = once(child, "exit");
    let output = "";
    child.stdout.on("data", bytes => output += bytes);
    while (!output.includes("ready")) await once(child.stdout, "data");
    child.kill("SIGTERM");
    assert.deepEqual(await exit, [0, null]);
    assert.match(output, /cleaned/);
  } finally {
    child?.kill("SIGTERM");
    await fs.rm(root, { recursive: true, force: true });
  }
});
