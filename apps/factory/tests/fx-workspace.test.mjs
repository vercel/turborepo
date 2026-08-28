import assert from "node:assert/strict";
import test from "node:test";

import { requireFactoryImage } from "../agent/lib/current-factory-image.ts";
import {
  WORKSPACE_DRIVE_CHECKOUT_PATH,
  WORKSPACE_DRIVE_FX_PATH,
  WORKSPACE_DRIVE_MOUNT_PATH,
  isWorkspaceDriveEnabled,
  workspaceDriveInitializationScript,
  workspaceDriveName
} from "../agent/lib/workspace-drive.ts";

test("workspace creation accepts the published Factory image", () => {
  const pointer = {
    buildId: "build-123",
    commit: "0123456789abcdef0123456789abcdef01234567",
    fingerprint: "factory-image",
    publishedAt: "2026-08-24T00:00:00.000Z",
    sandboxName: "factory-image-test",
    snapshotId: "snap_123",
    warmBuild: true
  };
  assert.equal(requireFactoryImage(pointer), pointer);
  assert.throws(() => requireFactoryImage(null), /has been published/);
});

test("workspace Drives are opt-in until the Vercel team has beta access", () => {
  assert.equal(isWorkspaceDriveEnabled(undefined), false);
  assert.equal(isWorkspaceDriveEnabled("0"), false);
  assert.equal(isWorkspaceDriveEnabled("1"), true);
});

test("Eve session drives keep the checkout and agent state together", () => {
  assert.equal(workspaceDriveName("wrun_abc"), "factory-eve-wrun_abc-drive");
  assert.equal(WORKSPACE_DRIVE_MOUNT_PATH, "/factory/persist");
  assert.equal(WORKSPACE_DRIVE_CHECKOUT_PATH, "/factory/persist/workspace");
  assert.equal(WORKSPACE_DRIVE_FX_PATH, "/factory/persist/fx");

  const script = workspaceDriveInitializationScript();
  assert.match(
    script,
    /cp -a \/factory\/turborepo\/\. \/factory\/persist\/workspace\//
  );
  assert.match(
    script,
    /ln -s \/factory\/persist\/workspace \/factory\/turborepo/
  );
  assert.match(script, /\.factory-initialized/);
});
