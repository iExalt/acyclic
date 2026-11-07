#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, rmdirSync, writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const FORMAT = 2;
const PROFILE = "production";
const BUILD_OPTIONS = "pnpm exec vite build --logLevel warn";
const SOURCE_INPUTS = [
  "mise.toml",
  "workspace/acyclic-app/package.json",
  "workspace/acyclic-app/pnpm-lock.yaml",
  "workspace/acyclic-app/vite.config.ts",
];
const LOCKFILE = "workspace/acyclic-app/pnpm-lock.yaml";

function fail(message) {
  throw new Error(message);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function stableJson(value) {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
  if (value && typeof value === "object")
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stableJson(value[key])}`).join(",")}}`;
  return JSON.stringify(value);
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value) &&
    (Object.getPrototypeOf(value) === Object.prototype || Object.getPrototypeOf(value) === null);
}

function checkedRegularFile(path, label) {
  let stat;
  try { stat = lstatSync(path); } catch { fail(`missing ${label}`); }
  if (!stat.isFile()) fail(`invalid ${label}`);
  return stat;
}

function checkedDirectory(path, label) {
  let stat;
  try { stat = lstatSync(path); } catch { fail(`missing ${label}`); }
  if (!stat.isDirectory()) fail(`invalid ${label}`);
}

function assertPath(rel) {
  if (
    typeof rel !== "string" || rel.length === 0 || rel.includes("\\") || rel.includes("\0") ||
    rel.startsWith("/") || rel.split("/").some((part) => part === "" || part === "." || part === "..") ||
    rel.normalize("NFC") !== rel
  ) fail("unsafe artifact path");
}

function sourceCommit(source) {
  const result = spawnSync("git", ["-C", source, "rev-parse", "--verify", "HEAD^{commit}"], {
    encoding: "utf8", timeout: 10000,
  });
  if (result.error || result.status !== 0) fail("cannot read source commit");
  const commit = result.stdout.trim();
  if (!/^[0-9a-f]{40,64}$/.test(commit)) fail("invalid source commit");
  const dirty = spawnSync("git", ["-C", source, "status", "--porcelain", "--untracked-files=all", "--", ".", ":!.build-tooling"], {
    encoding: "utf8", timeout: 10000,
  });
  if (dirty.error || dirty.status !== 0 || dirty.stdout.length !== 0) fail("source checkout is not clean");
  return commit;
}

function toolVersions(source) {
  const pnpm = spawnSync("pnpm", ["--version"], {
    cwd: source, encoding: "utf8", timeout: 10000, maxBuffer: 4096,
  });
  if (pnpm.error || pnpm.status !== 0) fail("cannot read effective pnpm version");
  const pnpmVersion = pnpm.stdout.trim();
  if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(pnpmVersion)) fail("invalid effective pnpm version");
  if (!/^v\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(process.version)) fail("invalid effective Node version");
  return { node: process.version, pnpm: pnpmVersion };
}

function sourceFingerprints(source) {
  const inputs = {};
  for (const rel of SOURCE_INPUTS) {
    assertPath(rel);
    checkedRegularFile(join(source, ...rel.split("/")), "source build input");
    inputs[rel] = sha256(readFileSync(join(source, ...rel.split("/"))));
  }
  return { inputs, lockfile: inputs[LOCKFILE] };
}

function collectFiles(dist) {
  checkedDirectory(dist, "dist directory");
  const files = [];
  const folded = new Set();

  function walk(dir, prefix) {
    for (const name of readdirSync(dir)) {
      const rel = prefix ? `${prefix}/${name}` : name;
      assertPath(rel);
      const key = rel.toLocaleLowerCase("en-US");
      if (folded.has(key)) fail("case-colliding artifact paths");
      folded.add(key);
      const path = join(dir, name);
      const stat = lstatSync(path);
      if (stat.isSymbolicLink()) fail("symlink in artifact");
      if (stat.isDirectory()) walk(path, rel);
      else if (stat.isFile()) files.push({ path: rel, size: stat.size, sha256: sha256(readFileSync(path)) });
      else fail("special file in artifact");
    }
  }

  walk(dist, "");
  files.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  const paths = new Set(files.map((file) => file.path));
  if (!paths.has("index.html")) fail("dist is missing index.html");
  if (!paths.has("licenses.txt")) fail("dist is missing licenses.txt");
  if (!paths.has("sw.js")) fail("dist is missing sw.js");
  if (!files.some((file) => /(^|\/)elk-worker\.min[^/]*\.js$/.test(file.path))) fail("dist is missing the ELK worker");
  if (!files.some((file) => file.path.endsWith(".woff2"))) fail("dist is missing its fonts");
  return files;
}

function bundleDigest(files) {
  return sha256(Buffer.from(stableJson(files), "utf8"));
}

export function artifactIdentity(source) {
  const repository = process.env.FRONTEND_TOOLING_REPOSITORY;
  const revision = process.env.FRONTEND_TOOLING_SHA;
  if (!/^[a-zA-Z0-9_.-]+\/[a-zA-Z0-9_.-]+$/.test(repository ?? "") ||
      !/^[0-9a-f]{40}$/.test(revision ?? "")) fail("missing defining workflow identity");
  const sourceInfo = sourceFingerprints(source);
  const identity = {
    format: FORMAT, profile: PROFILE, sourceCommit: sourceCommit(source),
    tooling: { repository, revision }, toolchain: toolVersions(source),
    build: { command: BUILD_OPTIONS, inputs: sourceInfo.inputs, lockfileSha256: sourceInfo.lockfile },
  };
  return { ...identity, fingerprint: sha256(Buffer.from(stableJson(identity))) };
}

function buildManifest(source, dist) {
  const files = collectFiles(dist);
  return { ...artifactIdentity(source), files, bundleSha256: bundleDigest(files) };
}

function assertArtifactRoot(root, forCreate) {
  if (existsSync(root)) {
    checkedDirectory(root, "artifact root");
    if (forCreate && readdirSync(root).length !== 0) fail("artifact root must be empty");
  } else if (forCreate) mkdirSync(root, { recursive: true });
  else fail("missing artifact root");
}

function copyTree(source, target) {
  checkedDirectory(source, "source dist directory");
  mkdirSync(target, { recursive: true });
  for (const name of readdirSync(source)) {
    const from = join(source, name);
    const to = join(target, name);
    const stat = lstatSync(from);
    if (stat.isSymbolicLink()) fail("symlink in source dist");
    if (stat.isDirectory()) copyTree(from, to);
    else if (stat.isFile()) {
      mkdirSync(dirname(to), { recursive: true });
      copyFileSync(from, to);
    } else fail("special file in source dist");
  }
}

function exactOuterContents(root) {
  const names = readdirSync(root).sort();
  if (stableJson(names) !== stableJson(["dist", "manifest.json"])) fail("unexpected artifact contents");
}

function create(source, artifactRoot) {
  assertArtifactRoot(artifactRoot, true);
  const stagedDist = join(artifactRoot, "dist");
  copyTree(join(source, "workspace/acyclic-app/dist"), stagedDist);
  const manifest = buildManifest(source, stagedDist);
  writeFileSync(join(artifactRoot, "manifest.json"), `${stableJson(manifest)}\n`, { flag: "wx" });
  exactOuterContents(artifactRoot);
}

function verify(source, artifactRoot) {
  assertArtifactRoot(artifactRoot, false);
  exactOuterContents(artifactRoot);
  checkedRegularFile(join(artifactRoot, "manifest.json"), "manifest");
  const parsed = JSON.parse(readFileSync(join(artifactRoot, "manifest.json"), "utf8"));
  if (!isPlainObject(parsed)) fail("invalid manifest shape");
  const actual = buildManifest(source, join(artifactRoot, "dist"));
  if (stableJson(parsed) !== stableJson(actual)) fail("manifest does not match source and artifact");
}

function install(source, artifactRoot) {
  verify(source, artifactRoot);
  const app = join(source, "workspace/acyclic-app");
  checkedDirectory(join(source, "workspace"), "source workspace");
  checkedDirectory(app, "source app");
  const target = join(app, "dist");
  if (existsSync(target)) {
    checkedDirectory(target, "destination dist");
    if (readdirSync(target).length !== 0) {
      if (stableJson(collectFiles(target)) !== stableJson(collectFiles(join(artifactRoot, "dist"))))
        fail("destination dist differs from the verified bundle");
      return;
    }
    rmdirSync(target);
  }
  copyTree(join(artifactRoot, "dist"), target);
}

function main() {
  const [operation, sourceArg, artifactArg, ...extra] = process.argv.slice(2);
  if (!["create", "verify", "install", "identity"].includes(operation) || !sourceArg || extra.length ||
      (operation === "identity" ? artifactArg : !artifactArg))
    fail("usage: frontend-artifact.mjs <create|verify|install> SOURCE_ROOT ARTIFACT_ROOT; identity SOURCE_ROOT");
  const source = resolve(sourceArg);
  checkedDirectory(source, "source root");
  if (operation === "identity") process.stdout.write(`${stableJson(artifactIdentity(source))}\n`);
  else if (operation === "create") create(source, resolve(artifactArg));
  else if (operation === "install") install(source, resolve(artifactArg));
  else verify(source, resolve(artifactArg));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) {
    process.stderr.write(`frontend-artifact: ${error.message}\n`);
    process.exitCode = 1;
  }
}
