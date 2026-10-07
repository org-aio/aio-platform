#!/usr/bin/env node
"use strict";

const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { PLATFORMS } = require("./release-config.cjs");

const packageRoot = path.resolve(__dirname, "..");
const rootManifest = JSON.parse(
  fs.readFileSync(path.join(packageRoot, "package.json"), "utf8")
);

function isMissingPackage(error) {
  try {
    return JSON.parse(error.stdout)?.error?.code === "E404";
  } catch {
    return false;
  }
}

function isPublished(name, version) {
  try {
    const published = execFileSync(
      "npm",
      ["view", `${name}@${version}`, "version", "--json"],
      { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }
    );
    return JSON.parse(published) === version;
  } catch (error) {
    if (isMissingPackage(error)) {
      return false;
    }
    throw error;
  }
}

function verifyRequiredDependencies(manifest, checkPublished = isPublished) {
  for (const [name, version] of Object.entries(manifest.dependencies ?? {})) {
    if (!checkPublished(name, version)) {
      throw new Error(`必须先发布依赖 ${name}@${version}，再发布 ${manifest.name}`);
    }
  }
}

function publish(directory, name, version) {
  if (isPublished(name, version)) {
    console.log(`已发布，跳过 ${name}@${version}`);
    return;
  }
  execFileSync("npm", ["publish", directory, "--access", "public", "--provenance"], {
    stdio: "inherit"
  });
}

function main() {
  const [, , platformRootArgument] = process.argv;
  if (platformRootArgument === undefined) {
    throw new Error("用法: publish-packages.cjs <平台包目录>");
  }
  const platformRoot = path.resolve(platformRootArgument);

  verifyRequiredDependencies(rootManifest);
  for (const platform of PLATFORMS) {
    publish(path.join(platformRoot, platform.id), platform.name, rootManifest.version);
  }
  publish(packageRoot, rootManifest.name, rootManifest.version);
}

if (require.main === module) {
  main();
}

module.exports = { isMissingPackage, verifyRequiredDependencies };
