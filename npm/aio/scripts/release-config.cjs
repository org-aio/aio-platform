"use strict";

const PLATFORMS = Object.freeze([
  {
    id: "darwin-arm64",
    name: "@zjarlin/aio-darwin-arm64",
    os: "darwin",
    cpu: "arm64",
    executable: "aio"
  },
  {
    id: "darwin-x64",
    name: "@zjarlin/aio-darwin-x64",
    os: "darwin",
    cpu: "x64",
    executable: "aio"
  },
  {
    id: "linux-arm64",
    name: "@zjarlin/aio-linux-arm64",
    os: "linux",
    cpu: "arm64",
    executable: "aio"
  },
  {
    id: "linux-x64",
    name: "@zjarlin/aio-linux-x64",
    os: "linux",
    cpu: "x64",
    executable: "aio"
  },
  {
    id: "win32-x64",
    name: "@zjarlin/aio-win32-x64",
    os: "win32",
    cpu: "x64",
    executable: "aio.exe"
  }
]);

module.exports = { PLATFORMS };
