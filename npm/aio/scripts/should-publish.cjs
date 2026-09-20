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

function isPublished(name, version) {
  try {
    const published = execFileSync(
      "npm",
      ["view", `${name}@${version}`, "version", "--json"],
      { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }
    );
    return JSON.parse(published) === version;
  } catch (error) {
    try {
      if (JSON.parse(error.stdout)?.error?.code === "E404") {
        return false;
      }
    } catch {
      // Fall through to the original registry failure below.
    }
    throw error;
  }
}

const missing = [rootManifest.name, ...PLATFORMS.map((platform) => platform.name)]
  .filter((name) => !isPublished(name, rootManifest.version));
const shouldPublish = missing.length > 0;

if (process.env.GITHUB_OUTPUT) {
  fs.appendFileSync(
    process.env.GITHUB_OUTPUT,
    `should_publish=${shouldPublish}\nmissing_packages=${missing.join(",")}\n`
  );
}

console.log(
  shouldPublish
    ? `需要发布 ${rootManifest.version}: ${missing.join(", ")}`
    : `npm 已存在 ${rootManifest.version}，无需发布`
);
