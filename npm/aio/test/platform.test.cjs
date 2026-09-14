"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const {
  PACKAGES,
  executableName,
  platformPackage
} = require("../lib/platform.cjs");

test("为全部发布目标选择平台包", () => {
  assert.deepEqual(PACKAGES, {
    "darwin-arm64": "@zjarlin/aio-darwin-arm64",
    "darwin-x64": "@zjarlin/aio-darwin-x64",
    "linux-arm64": "@zjarlin/aio-linux-arm64",
    "linux-x64": "@zjarlin/aio-linux-x64",
    "win32-x64": "@zjarlin/aio-win32-x64"
  });
  assert.equal(platformPackage("linux", "x64"), "@zjarlin/aio-linux-x64");
  assert.equal(platformPackage("darwin", "arm64"), "@zjarlin/aio-darwin-arm64");
});

test("Windows 使用 exe 后缀", () => {
  assert.equal(executableName("win32"), "aio.exe");
  assert.equal(executableName("linux"), "aio");
});

test("不支持的平台给出明确错误", () => {
  assert.throws(
    () => platformPackage("freebsd", "x64"),
    /AIO CLI 暂不支持 freebsd\/x64/
  );
});
