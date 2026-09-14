"use strict";

const PACKAGES = Object.freeze({
  "darwin-arm64": "@zjarlin/aio-darwin-arm64",
  "darwin-x64": "@zjarlin/aio-darwin-x64",
  "linux-arm64": "@zjarlin/aio-linux-arm64",
  "linux-x64": "@zjarlin/aio-linux-x64",
  "win32-x64": "@zjarlin/aio-win32-x64"
});

function platformPackage(platform = process.platform, arch = process.arch) {
  const key = `${platform}-${arch}`;
  const packageName = PACKAGES[key];
  if (packageName === undefined) {
    throw new Error(`AIO CLI 暂不支持 ${platform}/${arch}`);
  }
  return packageName;
}

function executableName(platform = process.platform) {
  return platform === "win32" ? "aio.exe" : "aio";
}

module.exports = { PACKAGES, executableName, platformPackage };
