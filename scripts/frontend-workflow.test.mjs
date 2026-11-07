import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import vm from "node:vm";

const root = fileURLToPath(new URL("../", import.meta.url));
const read = (name) => readFileSync(join(root, name), "utf8");
const build = read(".github/workflows/build.yml");
const ci = read(".github/workflows/ci.yml");
const archive = read(".github/actions/build-archive/action.yml");
const frontend = read(".github/actions/frontend/action.yml");
const publish = read(".github/workflows/publish.yml");
function job(workflow, name) {
  const match = workflow.match(new RegExp(`^  ${name}:\\n([\\s\\S]*?)(?=^  [a-z][a-z-]*:|$(?![\\s\\S]))`, "m"));
  assert.ok(match, `job ${name}`); return match[1];
}
const shell = (script, env) => spawnSync("bash", ["-e", "-o", "pipefail", "-c", script], {
  env: { ...process.env, ...env }, encoding: "utf8", timeout: 3000,
});

test("one parallel graph elects x86, preserves E2 and uses defining tooling after source tool selection", () => {
  assert.doesNotMatch(build, /^  (pull_request|push):/m);
  assert.match(build, /^  workflow_call:/m);
  assert.match(build, /permissions:\n  contents: read\n  actions: read/);
  assert.match(job(ci, "native-build"), /uses: \.\/\.github\/workflows\/build.yml/);
  assert.match(job(ci, "native-build"), /ref: \$\{\{ github.sha \}\}/);
  for (const block of [job(build, "build"), job(build, "build-macos"), job(ci, "browser")]) {
    assert.doesNotMatch(block, /^    needs:/m);

  }
});

test("source cache discovery precedes tooling checkout and exactly one native role can produce", () => {
  for (const block of [job(build, "build"), job(build, "build-macos"), job(ci, "browser")]) {
    assert.ok(block.indexOf("jdx/mise-action@") < block.indexOf("path: .build-tooling"));
    assert.match(block, /ref: \$\{\{ steps.tooling.outputs.revision \}\}/);
    assert.match(block, /FRONTEND_TOOLING_SHA: \$\{\{ steps.tooling.outputs.revision \}\}/);
  }
  assert.match(build, /frontend-producer: \$\{\{ matrix.arch == 'amd64' \}\}/);
  assert.equal((build.match(/frontend-producer:/g) ?? []).length, 1);
  assert.doesNotMatch(archive, /mise run build:app/);
  assert.ok(archive.indexOf(".github/actions/frontend") < archive.indexOf("cargo build --release"));
  assert.match(job(build, "rehearse"), /^    needs: build$/m);
  for (const text of [job(ci, "native-build"), job(ci, "browser"), job(publish, "build")])
    assert.match(text, /actions: read/);
  assert.match(job(publish, "release"), /packages: write/);
  assert.doesNotMatch(job(publish, "build"), /(?:contents|packages): write/);
});

test("only a producer without a committed bundle can build, create or upload", () => {
  const expressions = [...frontend.matchAll(/^      if: (inputs.producer.+)$/gm)].map((m) => m[1]);
  assert.equal(expressions.length, 3);
  for (const producer of ["true", "false"]) for (const id of ["", "17"]) {
    for (const expression of expressions) {
      const actual = expression.replaceAll("inputs.producer", "producer").replaceAll("steps.artifact.outputs.artifact-id", "id");
      assert.equal(vm.runInNewContext(actual, { producer, id }), producer === "true" && id === "");
    }
  }
  assert.doesNotMatch(frontend, /overwrite:|run-id:|github-token:/);
  assert.ok(frontend.indexOf("actions/upload-artifact@") < frontend.indexOf("install \"$GITHUB_WORKSPACE\""));
});

test("explicit CI notices policy preserves fresh external publishing notices", () => {
  assert.match(job(ci, "native-build"), /reuse-notices: true/);
  assert.doesNotMatch(job(publish, "build"), /reuse-notices: true/);
  assert.match(build, /reuse-notices:[\s\S]*?type: boolean\n        default: false/);
  const expr = build.match(/reuse-notices: \$\{\{ (.+) \}\}/)[1];
  for (const ref of ["", "synthetic-ref"]) for (const reuse of [false, true]) {
    assert.equal(vm.runInNewContext(expr.replaceAll("inputs.ref", "ref").replaceAll("inputs.reuse-notices", "reuse"), { ref, reuse }), ref === "" || reuse);
  }
});

test("actual inner and top-level aggregates reject every non-success required result", () => {
  const inner = job(build, "ok");
  assert.match(inner, /^    if: \$\{\{ always\(\) \}\}$/m);
  assert.match(inner, /^    needs: \[build, build-macos, rehearse\]$/m);
  const script = inner.match(/^      - run: \|\n((?:          .*\n?)*)/m)[1].replace(/^          /gm, "");
  const env = { BUILD: "success", BUILD_MACOS: "success", REHEARSE: "success", SKIP_REHEARSAL: "false" };
  assert.equal(shell(script, env).status, 0);
  for (const key of ["BUILD", "BUILD_MACOS", "REHEARSE"]) for (const value of ["failure", "cancelled", "skipped", "unknown", ""])
    assert.notEqual(shell(script, { ...env, [key]: value }).status, 0);
  assert.equal(shell(script, { ...env, REHEARSE: "skipped", SKIP_REHEARSAL: "true" }).status, 0);
  const outer = job(ci, "build-ok");
  assert.match(outer, /name: build ok\n/); assert.match(outer, /^    if: \$\{\{ always\(\) \}\}$/m);
  assert.match(outer, /^    needs: native-build$/m);
  const command = outer.match(/^      - run: (.+)$/m)[1];
  for (const result of ["success", "failure", "cancelled", "skipped", "unknown", ""])
    assert.equal(shell(command, { BUILD_RESULT: result }).status === 0, result === "success");
});

test("actual tooling extraction rejects absent, malformed and shell-bearing workflow identities", () => {
  const block = job(build, "build");
  const script = block.match(/        run: \|\n((?:          .*\n?)*)/m)[1].replace(/^          /gm, "");
  const valid = { workflow_repository: "example/synthetic", workflow_sha: "a".repeat(40) };
  assert.equal(shell(script, { JOB_CONTEXT: JSON.stringify(valid), GITHUB_OUTPUT: "/dev/null" }).status, 0);
  for (const context of [{}, { ...valid, workflow_sha: null }, { ...valid, workflow_repository: 7 },
    { ...valid, workflow_sha: "latest" }, { ...valid, workflow_repository: "a/b\nrevision=other" }])
    assert.notEqual(shell(script, { JOB_CONTEXT: JSON.stringify(context), GITHUB_OUTPUT: "/dev/null" }).status, 0);
});

test("real artifact browser entry executes the unchanged payload without any frontend build", (t) => {
  const dir = mkdtempSync(join(tmpdir(), "frontend-browser-")); t.after(() => rmSync(dir, { recursive: true }));
  const source = join(dir, "source"); const app = join(source, "workspace/acyclic-app");
  const dist = join(app, "dist"); const artifact = join(dir, "artifact"); const bin = join(dir, "bin");
  mkdirSync(join(dist, "assets"), { recursive: true }); mkdirSync(bin);
  for (const [name, bytes] of Object.entries({ "index.html": "synthetic", "licenses.txt": "synthetic", "sw.js": "synthetic",
    "assets/elk-worker.min-1.js": "synthetic", "assets/font.woff2": "synthetic" })) writeFileSync(join(dist, name), bytes);
  writeFileSync(join(source, "mise.toml"), "[tools]\n");
  for (const name of ["package.json", "pnpm-lock.yaml", "vite.config.ts"]) writeFileSync(join(app, name), "synthetic");
  writeFileSync(join(source, ".gitignore"), "workspace/acyclic-app/dist/\n");
  const git = (...args) => { const r = spawnSync("git", ["-C", source, ...args], { encoding: "utf8" }); assert.equal(r.status, 0, r.stderr); };
  git("init", "-q"); git("add", ".");
  git("-c", "user.name=Synthetic Fixture", "-c", "user.email=synthetic@example.invalid", "-c", "commit.gpgsign=false", "commit", "-qm", "fixture");
  for (const name of ["cargo", "uv"]) {
    writeFileSync(join(bin, name), `#!/bin/sh\nprintf '%s:%s:%s\\n' '${name}' "$PWD" "$*" >> "$PROBE_LOG"\n`);
    chmodSync(join(bin, name), 0o755);
  }
  const env = { ...process.env, FRONTEND_TOOLING_REPOSITORY: "example/synthetic", FRONTEND_TOOLING_SHA: "b".repeat(40), PROBE_LOG: join(dir, "commands") };
  const helper = join(root, "scripts/frontend-artifact.mjs");
  const create = spawnSync(process.execPath, [helper, "create", source, artifact], { env, encoding: "utf8" });
  assert.equal(create.status, 0, create.stderr); rmSync(dist, { recursive: true });
  const check = () => spawnSync("sh", [join(root, "scripts/check-browser-artifact.sh"), source, artifact], {
    env: { ...env, PATH: `${bin}:${env.PATH}` }, encoding: "utf8", timeout: 20000,
  });
  assert.equal(check().status, 0);
  const commands = readFileSync(env.PROBE_LOG, "utf8");
  assert.match(commands, /cargo:.*:build --quiet --locked --profile journeys -p acyclic -p acyclic-fixtures/);
  assert.equal((commands.match(/^uv:/gm) ?? []).length, 2);
  assert.doesNotMatch(commands, /build:app|vite|pnpm/);
  rmSync(env.PROBE_LOG); writeFileSync(join(artifact, "dist/index.html"), "tampered");
  assert.notEqual(check().status, 0);
  assert.throws(() => readFileSync(env.PROBE_LOG));
  const mise = read("mise.toml");
  assert.match(mise, /\[tasks\."check:browser"\][\s\S]*?depends = \["build:app"\]/);
  const consumer = mise.split('[tasks."check:browser:artifact"]')[1].split("\n[tasks.")[0];
  assert.doesNotMatch(consumer, /depends|build:app/);
});
