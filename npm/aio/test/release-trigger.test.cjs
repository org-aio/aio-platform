"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

test("发布工作流由 main 或版本标签推送触发并使用受保护的发布身份", () => {
  const workflow = fs.readFileSync(
    path.resolve(__dirname, "../../../.github/workflows/npm-release.yml"),
    "utf8"
  );

  assert.match(workflow, /branches:\s*\n\s*- "?main"?/);
  assert.match(workflow, /tags:\s*\n\s*- "v\*"/);
  assert.match(workflow, /should-publish\.cjs/);
  assert.match(workflow, /publish:\s*\n[\s\S]*?environment: npm/);
  assert.match(workflow, /NODE_AUTH_TOKEN: \$\{\{ secrets\.NPM_TOKEN \}\}/);
});

test("复用已验证产物时，发布任务不受构建 skipped 的隐式条件阻断", () => {
  const workflow = fs.readFileSync(
    path.resolve(__dirname, "../../../.github/workflows/npm-release.yml"),
    "utf8"
  );
  assert.match(workflow, /publish:\s*\n[\s\S]*?if: always\(\) && !cancelled\(\) && needs\.verify\.result == 'success'/);
  assert.match(workflow, /needs\.build\.result == 'skipped' && inputs\.artifacts_run_id != ''/);
  assert.match(workflow, /node npm\/aio\/scripts\/verify-reuse\.cjs/);
});
