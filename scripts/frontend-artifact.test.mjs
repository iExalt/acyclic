import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

process.env.FRONTEND_TOOLING_REPOSITORY = "example/synthetic";
process.env.FRONTEND_TOOLING_SHA = "b".repeat(40);

const helper = fileURLToPath(new URL("./frontend-artifact.mjs", import.meta.url));

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "frontend-artifact-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const source = join(root, "source");
  const app = join(source, "workspace/acyclic-app");
  const dist = join(app, "dist");
  mkdirSync(join(dist, "assets"), { recursive: true });
  writeFileSync(join(source, "mise.toml"), '[tools]\nnode = "26.12.1"\npnpm = "12.15.0"\n');
  writeFileSync(join(app, "package.json"), '{"scripts":{"build":"vite build"}}\n');
  writeFileSync(join(app, "pnpm-lock.yaml"), "lockfileVersion: '9.0'\n");
  writeFileSync(join(app, "vite.config.ts"), "export default {};\n");
  writeFileSync(join(dist, "index.html"), "<main>synthetic</main>\n");
  writeFileSync(join(dist, "licenses.txt"), "Synthetic license notice\n");
  writeFileSync(join(dist, "sw.js"), "self.addEventListener('fetch', () => {});\n");
  writeFileSync(join(dist, "assets/elk-worker.min-123.js"), "self.onmessage = () => {};\n");
  writeFileSync(join(dist, "assets/font.woff2"), Buffer.from([0, 1, 2, 3]));
  writeFileSync(join(dist, ".hidden"), "included\n");
  writeFileSync(join(dist, "assets/app.js.br"), Buffer.from([4, 5, 6]));
  writeFileSync(join(source, ".gitignore"), "workspace/acyclic-app/dist/\n");
  assert.equal(spawnSync("git", ["init", "-q", source]).status, 0);
  spawnSync("git", ["-C", source, "config", "core.excludesFile", "/dev/null"]);
  spawnSync("git", ["-C", source, "config", "user.email", "synthetic@example.invalid"]);
  spawnSync("git", ["-C", source, "config", "user.name", "Synthetic Fixture"]);
  spawnSync("git", ["-C", source, "add", "."]);
  assert.equal(spawnSync("git", ["-C", source, "-c", "commit.gpgsign=false", "commit", "-qm", "fixture"]).status, 0);
  const tooling = join(source, ".build-tooling");
  assert.equal(spawnSync("git", ["init", "-q", tooling]).status, 0);
  writeFileSync(join(tooling, "helper.mjs"), "synthetic tooling checkout\n");
  assert.equal(spawnSync("git", ["-C", source, "ls-files", ".build-tooling"], { encoding: "utf8" }).stdout, "");
  assert.match(spawnSync("git", ["-C", source, "status", "--porcelain"], { encoding: "utf8" }).stdout, /\?\? \.build-tooling\//);
  return { root, source, dist, artifact: join(root, "artifact") };
}

function run(command, source, artifact) {
  return spawnSync(process.execPath, [helper, command, source, ...(artifact === undefined ? [] : [artifact])], {
    cwd: source, encoding: "utf8", timeout: 20000,
  });
}

function create(t) {
  const paths = fixture(t);
  const result = run("create", paths.source, paths.artifact);
  assert.equal(result.status, 0, result.stderr);
  return paths;
}

test("manifest captures hidden and compressed files and verifies the staged tree", (t) => {
  const paths = create(t);
  const manifest = JSON.parse(readFileSync(join(paths.artifact, "manifest.json"), "utf8"));
  const names = manifest.files.map((file) => file.path);
  assert.ok(names.includes(".hidden"));
  assert.ok(names.includes("assets/app.js.br"));
  assert.equal(run("verify", paths.source, paths.artifact).status, 0);
});

test("verification rejects changed artifact bytes and additional files", (t) => {
  const paths = create(t);
  writeFileSync(join(paths.artifact, "dist/index.html"), "changed");
  assert.notEqual(run("verify", paths.source, paths.artifact).status, 0);
  writeFileSync(join(paths.artifact, "dist/index.html"), "<main>synthetic</main>\n");
  writeFileSync(join(paths.artifact, "dist/extra.js"), "extra");
  assert.notEqual(run("verify", paths.source, paths.artifact).status, 0);
});

test("verification rejects changed source build inputs and dirty checkouts", (t) => {
  const paths = create(t);
  writeFileSync(join(paths.source, "workspace/acyclic-app/vite.config.ts"), "export default { changed: true };\n");
  assert.notEqual(run("verify", paths.source, paths.artifact).status, 0);
});

test("creation and verification reject symlink roots and nested symlinks", (t) => {
  const paths = fixture(t);
  symlinkSync(paths.source, join(paths.root, "source-link"));
  assert.notEqual(run("create", join(paths.root, "source-link"), paths.artifact).status, 0);
  symlinkSync(paths.root, join(paths.root, "artifact-link"));
  assert.notEqual(run("verify", paths.source, join(paths.root, "artifact-link")).status, 0);
  symlinkSync(paths.root, join(paths.dist, "linked-dir"));
  assert.notEqual(run("create", paths.source, paths.artifact).status, 0);
});

test("creation rejects stale staging and incomplete production output", (t) => {
  const stale = fixture(t);
  mkdirSync(stale.artifact);
  writeFileSync(join(stale.artifact, "old.txt"), "old");
  assert.notEqual(run("create", stale.source, stale.artifact).status, 0);

  const incomplete = fixture(t);
  rmSync(join(incomplete.dist, "licenses.txt"));
  assert.notEqual(run("create", incomplete.source, incomplete.artifact).status, 0);
});

test("verification rejects a changed profile, committed source ref, and lockfile", (t) => {
  const profile = create(t);
  const manifestPath = join(profile.artifact, "manifest.json");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  manifest.profile = "development";
  writeFileSync(manifestPath, JSON.stringify(manifest));
  assert.notEqual(run("verify", profile.source, profile.artifact).status, 0);

  const tool = create(t);
  const toolPath = join(tool.artifact, "manifest.json");
  const toolManifest = JSON.parse(readFileSync(toolPath, "utf8"));
  toolManifest.toolchain.node = "v0.0.0";
  writeFileSync(toolPath, JSON.stringify(toolManifest));
  assert.notEqual(run("verify", tool.source, tool.artifact).status, 0);

  const ref = create(t);
  writeFileSync(join(ref.source, "source-marker"), "committed change\n");
  spawnSync("git", ["-C", ref.source, "add", "source-marker"]);
  assert.equal(spawnSync("git", ["-C", ref.source, "-c", "commit.gpgsign=false", "commit", "-qm", "new ref"]).status, 0);
  assert.notEqual(run("verify", ref.source, ref.artifact).status, 0);

  const lock = create(t);
  writeFileSync(join(lock.source, "workspace/acyclic-app/pnpm-lock.yaml"), "changed lockfile\n");
  spawnSync("git", ["-C", lock.source, "add", "workspace/acyclic-app/pnpm-lock.yaml"]);
  assert.equal(spawnSync("git", ["-C", lock.source, "-c", "commit.gpgsign=false", "commit", "-qm", "new lock"]).status, 0);
  assert.notEqual(run("verify", lock.source, lock.artifact).status, 0);
});

test("creation rejects unsafe paths", (t) => {
  const paths = fixture(t);
  writeFileSync(join(paths.dist, "unsafe\\name.js"), "unsafe");
  assert.notEqual(run("create", paths.source, paths.artifact).status, 0);
});

test("verification rejects malformed manifests and case-colliding output paths", (t) => {
  const paths = create(t);
  writeFileSync(join(paths.artifact, "manifest.json"), '{"files":[],"bundleSha256":[]}');
  assert.notEqual(run("verify", paths.source, paths.artifact).status, 0);
  writeFileSync(join(paths.artifact, "manifest.json"), "{}\n");
  writeFileSync(join(paths.artifact, "dist/Case.js"), "a");
  writeFileSync(join(paths.artifact, "dist/case.js"), "b");
  assert.notEqual(run("verify", paths.source, paths.artifact).status, 0);
});

test("verification rejects unlisted files beside the manifest and dirty sources", (t) => {
  const paths = create(t);
  writeFileSync(join(paths.artifact, "other"), "unexpected");
  assert.notEqual(run("verify", paths.source, paths.artifact).status, 0);
  rmSync(join(paths.artifact, "other"));
  writeFileSync(join(paths.source, "untracked-source.js"), "unexpected");
  assert.notEqual(run("verify", paths.source, paths.artifact).status, 0);
});

test("install reuses matching bytes, accepts an empty destination and rejects a stale destination", (t) => {
  const paths = create(t);
  assert.equal(run("install", paths.source, paths.artifact).status, 0);
  rmSync(paths.dist, { recursive: true });
  mkdirSync(paths.dist);
  assert.equal(run("install", paths.source, paths.artifact).status, 0);
  writeFileSync(join(paths.dist, "index.html"), "stale");
  assert.notEqual(run("install", paths.source, paths.artifact).status, 0);
});

test("identity changes with defining tooling and rejects missing identity", (t) => {
  const paths = fixture(t);
  const first = run("identity", paths.source);
  assert.equal(first.status, 0, first.stderr);
  const next = spawnSync(process.execPath, [helper, "identity", paths.source], {
    encoding: "utf8", env: { ...process.env, FRONTEND_TOOLING_SHA: "c".repeat(40) },
  });
  assert.equal(next.status, 0, next.stderr);
  assert.notEqual(JSON.parse(first.stdout).fingerprint, JSON.parse(next.stdout).fingerprint);
  const absent = spawnSync(process.execPath, [helper, "identity", paths.source], {
    encoding: "utf8", env: { ...process.env, FRONTEND_TOOLING_SHA: "" },
  });
  assert.notEqual(absent.status, 0);
});
