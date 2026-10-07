"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const path = require("node:path");
const os = require("node:os");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const test = require("node:test");
const { platformPackage, executableName } = require("../lib/platform.cjs");

test("aio device 从随包依赖启动，不需要全局设备命令或宿主二进制", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "aio-device-launcher-"));
  try {
    await fs.cp(path.resolve(__dirname, "../bin"), path.join(root, "bin"), { recursive: true });
    await fs.cp(path.resolve(__dirname, "../lib"), path.join(root, "lib"), { recursive: true });
    const entry = path.join(root, "node_modules/@zjarlin/aio-device/dist/cli.mjs");
    await fs.mkdir(path.dirname(entry), { recursive: true });
    await fs.writeFile(entry, "console.log(JSON.stringify(process.argv.slice(2)));process.exit(7);\n");
    const child = spawn(process.execPath, [path.join(root, "bin/aio.cjs"), "device", "workspace-add", "--name", "demo with spaces"], {
      env: { ...process.env, PATH: "" }, stdio: ["ignore", "pipe", "pipe"]
    });
    let output = "";
    child.stdout.on("data", bytes => output += bytes);
    assert.deepEqual(await once(child, "exit"), [7, null]);
    assert.deepEqual(JSON.parse(output), ["workspace-add", "--name", "demo with spaces"]);
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test("随包设备组件缺失时只提示修复统一入口", async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "aio-device-missing-"));
  try {
    await fs.cp(path.resolve(__dirname, "../bin"), path.join(root, "bin"), { recursive: true });
    await fs.cp(path.resolve(__dirname, "../lib"), path.join(root, "lib"), { recursive: true });
    const child = spawn(process.execPath, [path.join(root, "bin/aio.cjs"), "device", "--help"], { stdio: ["ignore", "pipe", "pipe"] });
    let error = "";
    child.stderr.on("data", bytes => error += bytes);
    assert.deepEqual(await once(child, "exit"), [1, null]);
    assert.match(error, /重新安装 @zjarlin\/aio/);
    assert.doesNotMatch(error, /npm install.*aio-device/);
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});

test("aio device 转发停止信号并等待设备组件清理", { skip: process.platform === "win32", timeout: 10000 }, async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "aio-device-signals-"));
  let child;
  try {
    await fs.cp(path.resolve(__dirname, "../bin"), path.join(root, "bin"), { recursive: true });
    await fs.cp(path.resolve(__dirname, "../lib"), path.join(root, "lib"), { recursive: true });
    const entry = path.join(root, "node_modules/@zjarlin/aio-device/dist/cli.mjs");
    await fs.mkdir(path.dirname(entry), { recursive: true });
    await fs.writeFile(entry, "process.on('SIGTERM',()=>setTimeout(()=>{console.log('cleaned');process.exit(0)},100));console.log('ready');setInterval(()=>{},1000);\n");
    child = spawn(process.execPath, [path.join(root, "bin/aio.cjs"), "device", "worker"], { stdio: ["ignore", "pipe", "pipe"] });
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
