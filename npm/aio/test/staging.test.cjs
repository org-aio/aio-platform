"use strict";

const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { PLATFORMS } = require("../scripts/release-config.cjs");

test("从各平台构建产物生成受限平台包", (context) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "aio-npm-test-"));
  context.after(() => fs.rmSync(root, { force: true, recursive: true }));
  const artifacts = path.join(root, "artifacts");
  const output = path.join(root, "packages");

  for (const platform of PLATFORMS) {
    const artifact = path.join(artifacts, platform.id, platform.executable);
    fs.mkdirSync(path.dirname(artifact), { recursive: true });
    fs.writeFileSync(artifact, platform.id);
    if (platform.developmentHost) {
      fs.writeFileSync(path.join(path.dirname(artifact), "aio-host"), platform.id);
      fs.mkdirSync(path.join(path.dirname(artifact), "web"));
      fs.writeFileSync(path.join(path.dirname(artifact), "web/index.html"), "Sandbox");
    }
  }

  execFileSync(
    process.execPath,
    [
      path.resolve(__dirname, "../scripts/stage-platform-packages.cjs"),
      artifacts,
      output
    ],
    { stdio: "inherit" }
  );

  for (const platform of PLATFORMS) {
    const packageRoot = path.join(output, platform.id);
    const manifest = JSON.parse(
      fs.readFileSync(path.join(packageRoot, "package.json"), "utf8")
    );
    assert.equal(manifest.name, platform.name);
    assert.deepEqual(manifest.os, [platform.os]);
    assert.deepEqual(manifest.cpu, [platform.cpu]);
    assert.deepEqual(manifest.files, ["bin", "LICENSE-APACHE", "LICENSE-MIT"]);
    assert.equal(
      fs.readFileSync(path.join(packageRoot, "LICENSE-APACHE"), "utf8"),
      fs.readFileSync(path.resolve(__dirname, "../LICENSE-APACHE"), "utf8")
    );
    assert.equal(
      fs.readFileSync(path.join(packageRoot, "LICENSE-MIT"), "utf8"),
      fs.readFileSync(path.resolve(__dirname, "../LICENSE-MIT"), "utf8")
    );
    assert.equal(
      fs.readFileSync(path.join(packageRoot, "bin", platform.executable), "utf8"),
      platform.id
    );
    const packed = JSON.parse(
      execFileSync("npm", ["pack", "--dry-run", "--json", packageRoot], {
        encoding: "utf8"
      })
    )[0];
    const packedFiles = packed.files.map((file) => file.path);
    assert.ok(packedFiles.includes(`bin/${platform.executable}`));
    assert.equal(packedFiles.includes("bin/aio-host"), Boolean(platform.developmentHost));
    assert.equal(packedFiles.includes("bin/web/index.html"), Boolean(platform.developmentHost));
    assert.ok(packedFiles.includes("LICENSE-APACHE"));
    assert.ok(packedFiles.includes("LICENSE-MIT"));
  }
});
