#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const { PLATFORMS } = require("./release-config.cjs");
const manifest = require("../package.json");
const repository = process.env.GITHUB_REPOSITORY;
const runId = process.env.ARTIFACTS_RUN_ID;
assert.match(repository || "", /^[\w.-]+\/[\w.-]+$/);
assert.match(runId || "", /^[1-9]\d*$/);

function api(path) {
  return JSON.parse(execFileSync("gh", ["api", `repos/${repository}/${path}`], { encoding: "utf8" }));
}

const run = api(`actions/runs/${runId}`);
assert.equal(run.repository.full_name, repository);
assert.equal(run.path, ".github/workflows/npm-release.yml");
assert.equal(run.status, "completed");
assert.match(run.head_sha, /^[a-f0-9]{40}$/);
execFileSync("git", ["merge-base", "--is-ancestor", run.head_sha, "HEAD"]);
const previous = JSON.parse(execFileSync("git", ["show", `${run.head_sha}:npm/aio/package.json`], { encoding: "utf8" }));
assert.equal(previous.version, manifest.version, "复用产物必须与发布版本一致");

// 仅重新包装 npm 文件；任何 Rust、前端、依赖或子模块变化都要求重新构建。
const changed = execFileSync("git", ["diff", "--name-only", run.head_sha, "HEAD"], { encoding: "utf8" }).trim().split("\n").filter(Boolean);
for (const file of changed) {
  assert.ok(file.startsWith("npm/aio/") || file === ".github/workflows/npm-release.yml", `构建输入已变化，不能复用：${file}`);
}
const jobs = api(`actions/runs/${runId}/jobs?per_page=100`).jobs;
const artifacts = api(`actions/runs/${runId}/artifacts?per_page=100`).artifacts;
for (const platform of PLATFORMS) {
  assert.ok(jobs.some(job => job.name === `构建 ${platform.id}` && job.conclusion === "success"), `${platform.id} 构建未通过`);
  assert.ok(artifacts.some(artifact => artifact.name === platform.id && !artifact.expired), `${platform.id} 产物不可用`);
}
console.log(`已验证可复用 ${repository} 运行 ${runId} 的 ${manifest.version} 构建产物，源码 ${run.head_sha}`);
