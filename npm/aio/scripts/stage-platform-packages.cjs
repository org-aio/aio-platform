#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { PLATFORMS } = require("./release-config.cjs");

const [, , artifactsArgument, outputArgument] = process.argv;
if (artifactsArgument === undefined || outputArgument === undefined) {
  throw new Error("用法: stage-platform-packages.cjs <artifacts目录> <输出目录>");
}

const packageRoot = path.resolve(__dirname, "..");
const artifactsRoot = path.resolve(artifactsArgument);
const outputRoot = path.resolve(outputArgument);
const rootManifest = JSON.parse(
  fs.readFileSync(path.join(packageRoot, "package.json"), "utf8")
);
const licenseFiles = ["LICENSE-APACHE", "LICENSE-MIT"];

fs.rmSync(outputRoot, { force: true, recursive: true });
fs.mkdirSync(outputRoot, { recursive: true });

for (const platform of PLATFORMS) {
  const source = path.join(artifactsRoot, platform.id, platform.executable);
  if (!fs.existsSync(source)) {
    throw new Error(`缺少 ${platform.id} 构建产物: ${source}`);
  }

  const destinationRoot = path.join(outputRoot, platform.id);
  const destinationBin = path.join(destinationRoot, "bin");
  fs.mkdirSync(destinationBin, { recursive: true });
  fs.copyFileSync(source, path.join(destinationBin, platform.executable));
  fs.chmodSync(path.join(destinationBin, platform.executable), 0o755);
  if (platform.developmentHost) {
    const host = path.join(artifactsRoot, platform.id, "aio-host");
    const web = path.join(artifactsRoot, platform.id, "web");
    if (!fs.existsSync(host) || !fs.existsSync(path.join(web, "index.html"))) {
      throw new Error(`缺少 ${platform.id} 配套开发宿主或 Web 产物`);
    }
    fs.copyFileSync(host, path.join(destinationBin, "aio-host"));
    fs.chmodSync(path.join(destinationBin, "aio-host"), 0o755);
    fs.cpSync(web, path.join(destinationBin, "web"), { recursive: true });
  }
  for (const licenseFile of licenseFiles) {
    fs.copyFileSync(
      path.join(packageRoot, licenseFile),
      path.join(destinationRoot, licenseFile)
    );
  }

  const manifest = {
    name: platform.name,
    version: rootManifest.version,
    description: `AIO CLI binary for ${platform.os}/${platform.cpu}`,
    license: rootManifest.license,
    repository: rootManifest.repository,
    os: [platform.os],
    cpu: [platform.cpu],
    files: ["bin", ...licenseFiles],
    preferUnplugged: true,
    publishConfig: rootManifest.publishConfig
  };
  fs.writeFileSync(
    path.join(destinationRoot, "package.json"),
    `${JSON.stringify(manifest, null, 2)}\n`
  );
  fs.writeFileSync(
    path.join(destinationRoot, "README.md"),
    `# ${platform.name}\n\n${rootManifest.name} 的 ${platform.os}/${platform.cpu} 预编译二进制。请安装 ${rootManifest.name}，不要直接依赖此包。\n`
  );
}
