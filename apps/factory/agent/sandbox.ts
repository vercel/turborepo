import { defineSandbox } from "eve/sandbox";
import { vercel } from "eve/sandbox/vercel";

import {
  readFactoryImageHandoff,
  writeFactoryImageHandoff
} from "./lib/factory-image-handoff";
import { readFactoryImagePointer } from "./lib/factory-image-registry";

/**
 * Sandbox for every Eve run in this app.
 *
 * Every agent starts directly from the latest published Factory image. Image
 * provisioning and verification happen only in the dedicated image-build
 * sandbox; sessions do not re-check tools, update the checkout, or install
 * dependencies before giving control to the agent.
 */

/** Matches fx workspaces, and leaves room for a `cargo build`. */
const SESSION_TIMEOUT_MS = 45 * 60 * 1000;
/** `.cargo/config.toml` builds with `-Zthreads=8`. */
const SESSION_VCPUS = 8;

export default defineSandbox({
  backend: () => {
    const handoff = readFactoryImageHandoff();
    if (handoff === null) {
      throw new Error(
        "No Factory image has been published. Build the shared image before starting agents."
      );
    }
    return vercel({
      resources: { vcpus: SESSION_VCPUS },
      source: { snapshotId: handoff.snapshotId, type: "snapshot" },
      timeout: SESSION_TIMEOUT_MS
    });
  },
  async bootstrap() {},
  revalidationKey: async () => {
    const pointer = await readFactoryImagePointer();
    writeFactoryImageHandoff({
      commit: pointer?.commit,
      snapshotId: pointer?.snapshotId
    });
    return `factory-image:${pointer?.snapshotId ?? "none"}`;
  }
});
