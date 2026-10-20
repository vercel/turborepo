import assert from "node:assert/strict";
import test from "node:test";

import {
  activeFactoryImageBuilds,
  beginFactoryImageProvisioning,
  beginFactoryImagePublication,
  claimFactoryImageBuild,
  EMPTY_FACTORY_IMAGE_STATE,
  factoryImageSandboxName,
  findFactoryImageBuild,
  isFactoryImageBuild,
  isFactoryImagePointer,
  parseFactoryImageState,
  publishFactoryImagePointer,
  updateFactoryImageBuild
} from "../agent/lib/factory-image-types.ts";

const FINGERPRINT = "1111222233334444";

function commit(seed) {
  return seed.repeat(40).slice(0, 40);
}

function claim(state, seed, buildId, overrides = {}) {
  return claimFactoryImageBuild(state, {
    buildId,
    commit: commit(seed),
    fingerprint: FINGERPRINT,
    now: `2026-08-19T00:00:0${buildId.length % 10}.000Z`,
    ref: "refs/heads/main",
    sandboxName: factoryImageSandboxName(commit(seed), buildId),
    trigger: "webhook",
    ...overrides
  });
}

function claimed(state, seed, buildId, overrides = {}) {
  const outcome = claim(state, seed, buildId, overrides);
  assert.equal(outcome.kind, "claimed");
  return outcome;
}

test("a claim queues one build with its own sandbox", () => {
  const first = claimed(EMPTY_FACTORY_IMAGE_STATE, "a", "build1");
  assert.equal(first.build.status, "queued");
  assert.equal(first.build.trigger, "webhook");
  assert.deepEqual(first.superseded, []);
  assert.equal(first.state.builds.length, 1);
  assert.equal(first.build.sandboxName, "factory-image-aaaaaaaaaaaa-build1");
});

test("a newer merge cancels every build still in flight", () => {
  const first = claimed(EMPTY_FACTORY_IMAGE_STATE, "a", "build1");
  const building = updateFactoryImageBuild(
    first.state,
    "build1",
    { status: "building" },
    "2026-08-19T00:01:00.000Z"
  );

  const second = claimed(building, "b", "build2");
  assert.deepEqual(
    second.superseded.map((build) => [
      build.id,
      build.status,
      build.supersededBy
    ]),
    [["build1", "cancelled", "build2"]]
  );
  assert.deepEqual(
    activeFactoryImageBuilds(second.state).map((build) => build.id),
    ["build2"]
  );
  assert.match(
    findFactoryImageBuild(second.state, "build1").message,
    /Superseded by bbbbbbb/
  );
});

test("a superseded build cannot resurrect itself", () => {
  const first = claimed(EMPTY_FACTORY_IMAGE_STATE, "a", "build1");
  const second = claimed(first.state, "b", "build2");

  // A step that was already in flight reports progress after losing.
  const late = updateFactoryImageBuild(
    second.state,
    "build1",
    { phase: "node-modules", status: "building" },
    "2026-08-19T00:02:00.000Z"
  );
  assert.equal(late, second.state);
  assert.equal(findFactoryImageBuild(late, "build1").status, "cancelled");

  // And it cannot publish over the winner either.
  const published = publishFactoryImagePointer(second.state, "build1", {
    now: "2026-08-19T00:03:00.000Z",
    snapshotId: "snap_loser",
    warmBuild: true
  });
  assert.equal(published.published, false);
  assert.equal(published.state.pointer, null);
});

test("redelivered webhooks reuse the live build", () => {
  const first = claimed(EMPTY_FACTORY_IMAGE_STATE, "a", "build1");
  const again = claim(first.state, "a", "build2");
  assert.equal(again.kind, "in-progress");
  assert.equal(again.build.id, "build1");
  assert.equal(activeFactoryImageBuilds(first.state).length, 1);
});

test("only one reconciler acquires provisioning and publication", () => {
  const first = claimed(EMPTY_FACTORY_IMAGE_STATE, "a", "build1");
  const provisioning = beginFactoryImageProvisioning(
    first.state,
    "build1",
    "2026-08-19T00:01:00.000Z"
  );
  assert.equal(provisioning.build.status, "building");
  assert.equal(provisioning.build.phase, "starting");
  assert.equal(
    beginFactoryImageProvisioning(
      provisioning.state,
      "build1",
      "2026-08-19T00:02:00.000Z"
    ).build,
    null
  );

  const publication = beginFactoryImagePublication(
    provisioning.state,
    "build1",
    "2026-08-19T00:03:00.000Z"
  );
  assert.equal(publication.build.status, "publishing");
  assert.equal(publication.build.phase, "snapshotting");
  assert.equal(
    beginFactoryImagePublication(
      publication.state,
      "build1",
      "2026-08-19T00:04:00.000Z"
    ).build,
    null
  );
});

test("a build that stopped reporting progress is replaced", () => {
  const first = claimed(EMPTY_FACTORY_IMAGE_STATE, "a", "build1");
  const stale = claim(first.state, "a", "build2", {
    now: "2026-08-19T01:00:00.000Z"
  });
  assert.equal(stale.kind, "claimed");
  assert.deepEqual(
    stale.superseded.map((build) => build.id),
    ["build1"]
  );
  assert.deepEqual(
    activeFactoryImageBuilds(stale.state).map((build) => build.id),
    ["build2"]
  );
});

test("a published image is not rebuilt for the same revision", () => {
  const first = claimed(EMPTY_FACTORY_IMAGE_STATE, "a", "build1");
  const published = publishFactoryImagePointer(first.state, "build1", {
    now: "2026-08-19T00:05:00.000Z",
    snapshotId: "snap_a",
    tools: { node: "v24.0.0" },
    warmBuild: true,
    warnings: ["could not install twiggy"]
  });
  assert.equal(published.published, true);
  assert.equal(published.state.pointer.snapshotId, "snap_a");
  assert.equal(published.state.pointer.commit, commit("a"));
  assert.deepEqual(published.state.pointer.tools, { node: "v24.0.0" });
  assert.equal(
    findFactoryImageBuild(published.state, "build1").status,
    "ready"
  );
  assert.match(
    findFactoryImageBuild(published.state, "build1").message,
    /1 warning/
  );

  const again = claim(published.state, "a", "build2");
  assert.equal(again.kind, "current");
  assert.equal(again.pointer.snapshotId, "snap_a");

  // A new toolchain rebuilds even at the same revision.
  const retooled = claim(published.state, "a", "build3", {
    fingerprint: "9999888877776666"
  });
  assert.equal(retooled.kind, "claimed");
});

test("the ledger tolerates unknown and malformed records", () => {
  const parsed = parseFactoryImageState({
    builds: [
      { id: "nope" },
      {
        commit: commit("c"),
        createdAt: "2026-08-19T00:00:00.000Z",
        fingerprint: FINGERPRINT,
        id: "build9",
        ref: "refs/heads/main",
        sandboxName: "factory-image-cccccccccccc-build9",
        status: "ready",
        trigger: "operator",
        updatedAt: "2026-08-19T00:00:00.000Z"
      }
    ],
    pointer: { snapshotId: "snap_a" }
  });
  assert.equal(parsed.builds.length, 1);
  assert.equal(parsed.builds[0].id, "build9");
  assert.equal(parsed.pointer, null);
  assert.deepEqual(parseFactoryImageState(null), EMPTY_FACTORY_IMAGE_STATE);
  assert.equal(isFactoryImageBuild({ status: "unknown" }), false);
  assert.equal(
    isFactoryImagePointer({
      buildId: "build1",
      commit: commit("a"),
      fingerprint: FINGERPRINT,
      publishedAt: "2026-08-19T00:00:00.000Z",
      sandboxName: "factory-image-aaaaaaaaaaaa-build1",
      snapshotId: "snap_a",
      warmBuild: false
    }),
    true
  );
});
