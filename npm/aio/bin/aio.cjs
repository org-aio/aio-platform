#!/usr/bin/env node
"use strict";

const { spawn } = require("node:child_process");
const { executableName, platformPackage } = require("../lib/platform.cjs");

let binary;
try {
  const packageName = platformPackage();
  binary = require.resolve(`${packageName}/bin/${executableName()}`);
} catch (error) {
  console.error(`无法启动 AIO CLI: ${error.message}`);
  console.error("请重新安装 @zjarlin/aio，并确认 npm 未禁用 optionalDependencies。");
  process.exitCode = 1;
  return;
}

const child = spawn(binary, process.argv.slice(2), { stdio: "inherit" });
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
