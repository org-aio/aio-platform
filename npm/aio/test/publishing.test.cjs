"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const {
  isMissingPackage,
  verifyRequiredDependencies
} = require("../scripts/publish-packages.cjs");
const manifest = require("../package.json");

function registryError(code) {
  return {
    stdout: JSON.stringify({ error: { code } })
  };
}

test("仅将 registry E404 识别为未发布", () => {
  assert.equal(isMissingPackage(registryError("E404")), true);
  assert.equal(isMissingPackage(registryError("E401")), false);
  assert.equal(isMissingPackage({ stdout: "invalid response" }), false);
});

test("设备组件尚未发布时阻止主 CLI 发布", () => {
  assert.throws(
    () => verifyRequiredDependencies(manifest, () => false),
    /必须先发布依赖 @zjarlin\/aio-device@0\.12\.1/
  );
});

test("所需的精确依赖版本已发布时允许继续", () => {
  const checked = [];
  verifyRequiredDependencies(manifest, (name, version) => {
    checked.push([name, version]);
    return true;
  });
  assert.deepEqual(checked, [["@zjarlin/aio-device", "0.12.1"]]);
});

test("依赖检查的授权和网络错误不会被当作未发布", () => {
  for (const code of ["E401", "ETIMEDOUT"]) {
    const error = registryError(code);
    assert.throws(
      () => verifyRequiredDependencies(manifest, () => { throw error; }),
      (actual) => actual === error
    );
  }
});
