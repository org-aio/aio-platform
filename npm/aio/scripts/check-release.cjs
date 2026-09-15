#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { PLATFORMS } = require("./release-config.cjs");

const packageRoot = path.resolve(__dirname, "..");
const repositoryRoot = path.resolve(packageRoot, "../..");
const manifest = JSON.parse(
  fs.readFileSync(path.join(packageRoot, "package.json"), "utf8")
);
const cargo = fs.readFileSync(path.join(repositoryRoot, "cli/Cargo.toml"), "utf8");
const cargoVersion = cargo.match(/\[package\][\s\S]*?\nversion = "([^"]+)"/)?.[1];

if (cargoVersion === undefined) {
  throw new Error("无法读取 Cargo CLI 版本");
}
if (manifest.version !== cargoVersion) {
  throw new Error(`npm 版本 ${manifest.version} 与 Cargo 版本 ${cargoVersion} 不一致`);
}

for (const file of ["host/Cargo.toml", "lib/plugin/host/Cargo.toml", "lib/plugin/development/Cargo.toml"]) {
  const content = fs.readFileSync(path.join(repositoryRoot, file), "utf8");
  const version = content.match(/\[package\][\s\S]*?\nversion = "([^"]+)"/)?.[1];
  if (version !== manifest.version) {
    throw new Error(`${file} 版本 ${version} 与 CLI ${manifest.version} 不一致`);
  }
}

for (const platform of PLATFORMS) {
  if (manifest.optionalDependencies[platform.name] !== manifest.version) {
    throw new Error(`${platform.name} 必须固定为 ${manifest.version}`);
  }
}

const tag = process.argv[2];
if (tag !== undefined && tag !== `v${manifest.version}`) {
  throw new Error(`发布标签 ${tag} 必须等于 v${manifest.version}`);
}

console.log(`AIO npm 发布配置有效: ${manifest.version}`);
