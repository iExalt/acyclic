import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import vm from 'node:vm';

const workflow = readFileSync(new URL('../.github/workflows/build.yml', import.meta.url), 'utf8');
const archive = readFileSync(new URL('../.github/actions/build-archive/action.yml', import.meta.url), 'utf8');
const producer = readFileSync(new URL('../.github/workflows/frontend.yml', import.meta.url), 'utf8');
function job(name) {
  const match = workflow.match(new RegExp(`^  ${name}:\\n([\\s\\S]*?)(?=^  [a-z][a-z-]*:|$(?![\\s\\S]))`, 'm'));
  assert.ok(match, `job ${name} exists`);
  return match[1];
}

test('native jobs consume a successful producer or an explicitly supplied same-run artifact', () => {
  for (const name of ['build', 'build-macos']) {
    const block = job(name);
    assert.match(block, /^    needs: frontend$/m);
    const expression = block.match(/^    if: \$\{\{ (.+) \}\}$/m)?.[1];
    assert.ok(expression);
    const actual = expression.replaceAll('always()', 'true')
      .replaceAll('needs.frontend.result', 'result')
      .replaceAll('inputs.frontend-artifact-id', 'artifact');
    for (const result of ['success', 'failure', 'cancelled', 'skipped', 'unknown']) {
      for (const artifact of ['', '123']) {
        assert.equal(vm.runInNewContext(actual, { result, artifact }),
          result === 'success' || (result === 'skipped' && artifact !== ''), `${name}: ${result}/${artifact}`);
      }
    }
    assert.match(block, /uses: \.\/\.build-tooling\/\.github\/actions\/build-archive/);
    assert.match(block, /frontend-artifact-id: \$\{\{ inputs.frontend-artifact-id \|\| needs.frontend.outputs.artifact-id \}\}/);
    assert.match(block, /reuse-notices: \$\{\{ inputs.ref == '' \}\}/);
  }
  assert.match(job('image'), /^    needs: build$/m);
});

test('actual aggregate rejects every non-success result except explicitly allowed skips', () => {
  const block = job('ok');
  assert.match(block, /^    if: \$\{\{ always\(\) \}\}$/m);
  assert.match(block, /^    needs: \[frontend, build, build-macos, image, rehearse\]$/m);
  const script = block.match(/^      - run: \|\n((?:          .*\n?)*)/m)?.[1]?.replace(/^          /gm, '');
  assert.ok(script);
  const baseline = { FRONTEND: 'success', FRONTEND_ARTIFACT_ID: '', BUILD: 'success', BUILD_MACOS: 'success', IMAGE: 'success', REHEARSE: 'success', SKIP_REHEARSAL: 'false' };
  const run = (delta) => spawnSync('bash', ['-e', '-c', script], { env: { ...process.env, ...baseline, ...delta }, timeout: 3000 }).status;
  assert.equal(run({}), 0);
  for (const key of ['FRONTEND', 'BUILD', 'BUILD_MACOS', 'IMAGE', 'REHEARSE']) {
    for (const result of ['failure', 'cancelled', 'skipped', 'unknown', '']) {
      assert.notEqual(run({ [key]: result }), 0, `${key}=${result}`);
    }
  }
  assert.equal(run({ FRONTEND: 'skipped', FRONTEND_ARTIFACT_ID: '123' }), 0);
  assert.equal(run({ REHEARSE: 'skipped', SKIP_REHEARSAL: 'true' }), 0);
  for (const result of ['failure', 'cancelled', 'unknown', '']) {
    assert.notEqual(run({ REHEARSE: result, SKIP_REHEARSAL: 'true' }), 0);
    assert.notEqual(run({ FRONTEND: result, FRONTEND_ARTIFACT_ID: '123' }), 0);
  }
});

test('tooling provenance and standalone frontend production stay separate from source checkout', () => {
  for (const block of [job('build'), job('build-macos'), producer]) {
    assert.match(block, /repository: \$\{\{ steps.tooling.outputs.repository \}\}/);
    assert.match(block, /ref: \$\{\{ steps.tooling.outputs.revision \}\}/);
    assert.ok(block.indexOf('Require defining workflow identity') < block.indexOf("Check out the defining workflow's tooling"));
  }
  assert.match(job('frontend'), /if: inputs.frontend-artifact-id == ''/);
  assert.equal((producer.match(/run: mise run build:app/g) ?? []).length, 1);
  assert.doesNotMatch(archive, /mise run build:app/);
  assert.match(archive, /artifact-ids: \$\{\{ inputs.frontend-artifact-id \}\}/);
  assert.doesNotMatch(archive, /run-id:|github-token:/);
  assert.ok(archive.indexOf('frontend-artifact.mjs" verify') < archive.indexOf('rm -rf workspace/acyclic-app/dist'));
});


test('actual provenance extraction rejects missing or malformed identity before checkout', () => {
  const block = job('build');
  const script = block.match(/        run: \|\n((?:          .*\n?)*)/m)?.[1]?.replace(/^          /gm, '');
  assert.ok(script);
  const run = (context) => spawnSync('bash', ['-e', '-c', script], {
    env: { ...process.env, JOB_CONTEXT: JSON.stringify(context), GITHUB_OUTPUT: '/dev/null' }, timeout: 3000,
  }).status;
  const valid = { workflow_repository: 'example/project', workflow_sha: 'a'.repeat(40) };
  assert.equal(run(valid), 0);
  for (const context of [{}, { ...valid, workflow_sha: null }, { ...valid, workflow_repository: 7 },
    { ...valid, workflow_sha: 'latest' }, { ...valid, workflow_repository: 'a/b\nrevision=evil' },
    { ...valid, workflow_repository: 'a/b/c' }]) assert.notEqual(run(context), 0);
});


test('source-selected tools are cached before the tooling checkout, with a separate producer tool set', () => {
  const setup = 'uses: jdx/mise-action@08ae2a091c038e1e05ec6f5115dc5c69d5a2a6b8';
  for (const block of [job('build'), job('build-macos'), producer]) {
    assert.equal(block.split(setup).length - 1, 1);
    assert.ok(block.indexOf(setup) < block.indexOf("Check out the defining workflow's tooling"));
  }
  assert.doesNotMatch(archive, /jdx\/mise-action/);
  assert.match(producer, /install_args: node pnpm/);
  for (const name of ['build', 'build-macos']) {
    assert.match(job(name), /install_args: rust node pnpm github:EmbarkStudios\/cargo-about/);
    assert.doesNotMatch(job(name), /cache_key:/);
  }
});
