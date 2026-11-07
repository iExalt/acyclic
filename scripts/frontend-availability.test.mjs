import assert from "node:assert/strict";
import test from "node:test";
import { artifactName, resolveArtifact } from "./frontend-availability.mjs";

const request = {
  repository: "example/synthetic", repositoryId: "3", runId: "12", token: "synthetic-token", wait: false,
  identity: { sourceCommit: "a".repeat(40), fingerprint: "b".repeat(64) },
};
const artifact = {
  id: 7, name: artifactName(request.runId, request.identity), expired: false, digest: `sha256:${"c".repeat(64)}`,
  workflow_run: { id: 12, repository_id: 3 }, url: "https://api.github.com/repos/example/synthetic/actions/artifacts/7",
};
function response(artifacts) { return Response.json({ total_count: artifacts.length, artifacts }); }

test("producer absent lookup returns immediately; consumers wait for exact immutable identity", async () => {
  let calls = 0;
  const absent = await resolveArtifact(request, { fetchImpl: async () => { calls++; return response([]); } });
  assert.equal(absent.id, ""); assert.equal(calls, 1);
  let clock = 0;
  const ready = await resolveArtifact({ ...request, wait: true }, {
    now: () => clock, sleep: async (ms) => { clock += ms; },
    fetchImpl: async (url, options) => {
      assert.ok(url.includes(`/runs/12/artifacts?name=${artifact.name}`));
      assert.equal(options.redirect, "error");
      calls++;
      return response(calls < 4 ? [] : [artifact]);
    },
  });
  assert.equal(ready.id, "7"); assert.equal(clock, 10000);
});

test("same identity is reusable across attempts, while source/run/tooling identities differ", async () => {
  const fetchImpl = async () => response([artifact]);
  assert.equal((await resolveArtifact(request, { fetchImpl })).id, "7");
  assert.equal((await resolveArtifact(request, { fetchImpl })).id, "7");
  for (const mutation of [
    { runId: "13" }, { identity: { ...request.identity, sourceCommit: "d".repeat(40) } },
    { identity: { ...request.identity, fingerprint: "e".repeat(64) } },
  ]) await assert.rejects(resolveArtifact({ ...request, ...mutation }, { fetchImpl }));
});

test("duplicate, expired, wrong repository/run, malformed and digest-less artifacts fail closed", async () => {
  for (const value of [
    [artifact, artifact], [{ ...artifact, expired: true }], [{ ...artifact, id: "7" }],
    [{ ...artifact, name: "other" }], [{ ...artifact, digest: undefined }],
    [{ ...artifact, workflow_run: { id: 13, repository_id: 3 } }],
    [{ ...artifact, workflow_run: { id: 12, repository_id: 4 } }],
    [{ ...artifact, url: "https://example.invalid/redirect" }],
  ]) await assert.rejects(resolveArtifact(request, { fetchImpl: async () => response(value) }));
  for (const body of [null, {}, { total_count: 1, artifacts: [] }, { total_count: 0, artifacts: "bad" }])
    await assert.rejects(resolveArtifact(request, { fetchImpl: async () => Response.json(body) }));
});

test("authorization and HTTP failures stop immediately without retry or alternate credential", async () => {
  for (const code of [401, 403, 404, 429, 500]) {
    let calls = 0;
    await assert.rejects(resolveArtifact({ ...request, wait: true }, {
      fetchImpl: async () => { calls++; return new Response("synthetic failure", { status: code }); },
    }));
    assert.equal(calls, 1);
  }
});

test("absence is bounded including the final polling interval", async () => {
  let clock = 0;
  await assert.rejects(resolveArtifact({ ...request, wait: true }, {
    deadlineMs: 12, now: () => clock, sleep: async (ms) => { clock += ms; },
    fetchImpl: async () => response([]),
  }), /deadline/);
  assert.equal(clock, 12);
});

test("deadline covers stalled response headers AND body; oversized and invalid JSON fail", async () => {
  const never = () => new Promise(() => {});
  for (const fetchImpl of [never, async () => ({ status: 200, body: { getReader: () => ({ read: never }) } })])
    await assert.rejects(resolveArtifact(request, { fetchImpl, deadlineMs: 15, requestMs: 10 }), /deadline/);
  for (const body of ["not JSON", "x".repeat(262145)])
    await assert.rejects(resolveArtifact(request, { fetchImpl: async () => new Response(body) }));
});
