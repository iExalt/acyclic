#!/usr/bin/env node
import { appendFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { artifactIdentity } from "./frontend-artifact.mjs";

class AvailabilityError extends Error {}

export function artifactName(runId, identity) {
  if (!/^[1-9][0-9]*$/.test(runId) || !/^[0-9a-f]{40,64}$/.test(identity.sourceCommit) ||
      !/^[0-9a-f]{64}$/.test(identity.fingerprint)) throw new AvailabilityError("invalid frontend identity");
  return `frontend-${runId}-${identity.sourceCommit}-${identity.fingerprint}`;
}

async function requestArtifacts(url, token, fetchImpl, timeoutMs) {
  const controller = new AbortController();
  let timer;
  try {
    return await Promise.race([
      (async () => {
        const response = await fetchImpl(url, {
          headers: { Authorization: `Bearer ${token}`, Accept: "application/vnd.github+json" },
          redirect: "error", signal: controller.signal,
        });
        if (response.status !== 200) throw new AvailabilityError(`artifact API returned HTTP ${response.status}`);
        const reader = response.body.getReader();
        let text = "";
        const decoder = new TextDecoder("utf-8", { fatal: true });
        let size = 0;
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          size += value.byteLength;
          if (size > 262144) { await reader.cancel(); throw new AvailabilityError("artifact response is too large"); }
          text += decoder.decode(value, { stream: true });
        }
        text += decoder.decode();
        return JSON.parse(text);
      })(),
      new Promise((_, reject) => {
        timer = setTimeout(() => { controller.abort(); reject(new AvailabilityError("artifact request deadline exceeded")); }, timeoutMs);
      }),
    ]);
  } catch (error) {
    if (error instanceof AvailabilityError) throw error;
    throw new AvailabilityError("artifact response or transport failed");
  } finally { clearTimeout(timer); controller.abort(); }
}

export async function resolveArtifact({ repository, repositoryId, runId, identity, token, wait }, {
  fetchImpl = fetch, now = () => performance.now(),
  sleep = (ms) => new Promise((resolveSleep) => setTimeout(resolveSleep, ms)),
  deadlineMs = 120000, requestMs = 10000,
} = {}) {
  if (!/^[a-zA-Z0-9_.-]+\/[a-zA-Z0-9_.-]+$/.test(repository) ||
      !/^[1-9][0-9]*$/.test(repositoryId) || !token || typeof wait !== "boolean")
    throw new AvailabilityError("missing artifact repository, token or operation");
  const name = artifactName(runId, identity);
  const deadline = now() + deadlineMs;
  const url = `https://api.github.com/repos/${repository}/actions/runs/${runId}/artifacts?name=${encodeURIComponent(name)}&per_page=100`;
  for (;;) {
    const remaining = deadline - now();
    if (remaining <= 0) throw new AvailabilityError(`frontend unavailable before deadline for run ${runId}: ${name}`);
    const data = await requestArtifacts(url, token, fetchImpl, Math.min(requestMs, remaining));
    if (now() >= deadline) throw new AvailabilityError("frontend availability deadline exceeded");
    if (!data || !Number.isSafeInteger(data.total_count) || !Array.isArray(data.artifacts) ||
        data.total_count !== data.artifacts.length || data.total_count < 0)
      throw new AvailabilityError("malformed artifact response");
    if (data.total_count > 1) throw new AvailabilityError("ambiguous frontend artifacts");
    if (data.total_count === 1) {
      const artifact = data.artifacts[0];
      if (!Number.isSafeInteger(artifact.id) || artifact.id <= 0 || artifact.name !== name ||
          artifact.expired !== false || !/^sha256:[0-9a-f]{64}$/.test(artifact.digest ?? "") ||
          String(artifact.workflow_run?.id) !== runId || String(artifact.workflow_run?.repository_id) !== repositoryId ||
          artifact.url !== `https://api.github.com/repos/${repository}/actions/artifacts/${artifact.id}`)
        throw new AvailabilityError("artifact does not match the exact run and frontend identity");
      return { name, id: String(artifact.id), digest: artifact.digest };
    }
    if (!wait) return { name, id: "", digest: "" };
    await sleep(Math.min(5000, deadline - now()));
  }
}

async function main() {
  const started = performance.now();
  const [operation, source, ...extra] = process.argv.slice(2);
  if (!["lookup", "wait"].includes(operation) || !source || extra.length)
    throw new AvailabilityError("usage: frontend-availability.mjs <lookup|wait> SOURCE_ROOT");
  const result = await resolveArtifact({
    repository: process.env.GITHUB_REPOSITORY, repositoryId: process.env.GITHUB_REPOSITORY_ID,
    runId: process.env.GITHUB_RUN_ID, identity: artifactIdentity(resolve(source)),
    token: process.env.GH_TOKEN, wait: operation === "wait",
  }, { deadlineMs: 120000 - (performance.now() - started) });
  if (!process.env.GITHUB_OUTPUT) throw new AvailabilityError("missing action output file");
  appendFileSync(process.env.GITHUB_OUTPUT,
    `artifact-name=${result.name}\nartifact-id=${result.id}\nartifact-digest=${result.digest}\n`);
  console.log(`frontend ${operation}: ${result.id ? `artifact ${result.id}` : "not published"}; ${Math.round(performance.now() - started)} ms; ${new Date().toISOString()}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    // Never echo request objects, bearer credentials or arbitrary network response bodies.
    process.stderr.write(`frontend availability: ${error instanceof AvailabilityError ? error.message : "frontend identity unavailable"}\n`);
    process.exitCode = 1;
  });
}
