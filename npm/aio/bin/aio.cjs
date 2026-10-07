#!/usr/bin/env node
"use strict";

const { spawn } = require("node:child_process");
const { executableName, platformPackage } = require("../lib/platform.cjs");

let binary;
let arguments_;
try {
  if (process.argv[2] === "device") {
    // 从随包依赖启动设备组件，不依赖另一个全局 CLI 或 PATH 配置。
    binary = process.execPath;
    arguments_ = [require.resolve("@zjarlin/aio-device/dist/cli.mjs"), ...process.argv.slice(3)];
  } else {
    const packageName = platformPackage();
    binary = require.resolve(`${packageName}/bin/${executableName()}`);
    arguments_ = process.argv.slice(2);
  }
} catch (error) {
  console.error(`无法启动 AIO CLI: ${error.message}`);
  console.error(process.argv[2] === "device"
    ? "请重新安装 @zjarlin/aio，以恢复随包设备组件。"
    : "请重新安装 @zjarlin/aio，并确认 npm 未禁用 optionalDependencies。");
  process.exitCode = 1;
  return;
}

const child = spawn(binary, arguments_, { stdio: "inherit" });
const signals = ["SIGINT", "SIGTERM"];
for (const signal of signals) process.on(signal, () => child.kill(signal));
child.once("error", (error) => {
  console.error(`无法启动 AIO CLI: ${error.message}`);
  process.exitCode = 1;
});
child.once("exit", (code, signal) => {
  if (signal !== null) {
    for (const name of signals) process.removeAllListeners(name);
    process.kill(process.pid, signal);
    return;
  }
  process.exitCode = code ?? 1;
});
