import { Drive } from "@vercel/sandbox";
import { defineSandbox } from "eve/sandbox";
import { vercel } from "eve/sandbox/vercel";

import {
  readFactoryImageHandoff,
  writeFactoryImageHandoff
} from "./lib/factory-image-handoff";
import { readFactoryImagePointer } from "./lib/factory-image-registry";
import {
  isWorkspaceDriveEnabled,
  WORKSPACE_DRIVE_MOUNT_PATH,
  workspaceDriveInitializationScript,
  workspaceDriveName
} from "./lib/workspace-drive";

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
      async sessionCreateOptions({ session }) {
        if (!isWorkspaceDriveEnabled()) return {};
        const drive = await Drive.getOrCreate({
          name: workspaceDriveName(session.id)
        });
        return {
          mounts: {
            [WORKSPACE_DRIVE_MOUNT_PATH]: {
              drive: drive.name,
              mode: "read-write"
            }
          }
        };
      },
      source: { snapshotId: handoff.snapshotId, type: "snapshot" },
      timeout: SESSION_TIMEOUT_MS
    });
  },
  async bootstrap() {},
  async onSession({ use }) {
    if (!isWorkspaceDriveEnabled()) return;
    const sandbox = await use();
    const result = await sandbox.run({
      command: `bash -lc ${JSON.stringify(workspaceDriveInitializationScript())}`
    });
    if (result.exitCode !== 0) {
      throw new Error(result.stderr || "Could not initialize workspace Drive.");
    }
  },
  revalidationKey: async () => {
    const pointer = await readFactoryImagePointer();
    writeFactoryImageHandoff({
      commit: pointer?.commit,
      fingerprint: pointer?.fingerprint,
      snapshotId: pointer?.snapshotId
    });
    return `factory-image:${pointer?.snapshotId ?? "none"}`;
  }
});
