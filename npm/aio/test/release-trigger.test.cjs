"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

test("发布工作流由 main 或版本标签推送自动触发且不再要求人工审批", () => {
  const workflow = fs.readFileSync(
    path.resolve(__dirname, "../../../.github/workflows/npm-release.yml"),
    "utf8"
  );

  assert.match(workflow, /branches:\s*\n\s*- "?main"?/);
  assert.match(workflow, /tags:\s*\n\s*- "v\*"/);
  assert.match(workflow, /should-publish\.cjs/);
  assert.doesNotMatch(workflow, /environment: npm/);
  assert.doesNotMatch(workflow, /required reviewers|required_reviewers|人工审批/);
});
