import assert from "node:assert/strict";
import { readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

const handoffPath = path.join(
  tmpdir(),
  `turborepo-factory-image-base-${crypto.randomUUID()}.json`
);

async function freshModule() {
  process.env.FACTORY_IMAGE_HANDOFF_PATH = handoffPath;
  return import(
    `../agent/lib/factory-image-handoff.ts?test=${crypto.randomUUID()}`
  );
}

test.after(() => {
  delete process.env.FACTORY_IMAGE_HANDOFF_PATH;
  rmSync(handoffPath, { force: true });
});

test("an environment-less build does not erase the published image handoff", async () => {
  const handoff = await freshModule();
  handoff.writeFactoryImageHandoff({
    commit: "0123456789abcdef0123456789abcdef01234567",
    fingerprint: "current-fingerprint",
    snapshotId: "snap_published"
  });
  handoff.writeFactoryImageHandoff({});

  assert.deepEqual(handoff.readFactoryImageHandoff(), {
    commit: "0123456789abcdef0123456789abcdef01234567",
    fingerprint: "current-fingerprint",
    snapshotId: "snap_published"
  });
  assert.equal(
    readFileSync(handoffPath, "utf8"),
    '{"commit":"0123456789abcdef0123456789abcdef01234567","fingerprint":"current-fingerprint","snapshotId":"snap_published"}'
  );
});
