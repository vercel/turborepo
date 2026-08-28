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
  workspaceCheckoutRefreshScript,
  workspaceDriveInitializationScript,
  workspaceDriveName
} from "./lib/workspace-drive";

/**
 * Sandbox for every Eve run in this app.
 *
 * Every agent starts directly from the latest published Factory image. Image
 * provisioning and verification happen only in the dedicated image-build
 * sandbox. New sessions update the checkout to current main, but do not
 * re-check tools or install dependencies before giving control to the agent.
 */

/** Matches fx workspaces, and leaves room for a `cargo build`. */
const SESSION_TIMEOUT_MS = 45 * 60 * 1000;
/** `.cargo/config.toml` builds with `-Zthreads=8`. */
const SESSION_VCPUS = 8;

export default defineSandbox({
  backend: () => {
    const handoff = readFactoryImageHandoff();
    const sessionCreateOptions = async ({
      session
    }: {
      session: { id: string };
    }) => {
      if (!isWorkspaceDriveEnabled()) return {};
      const drive = await Drive.getOrCreate({
        name: workspaceDriveName(session.id)
      });
      return {
        mounts: {
          [WORKSPACE_DRIVE_MOUNT_PATH]: {
            drive: drive.name,
            mode: "read-write" as const
          }
        }
      };
    };
    return vercel(
      handoff === null
        ? {
            resources: { vcpus: SESSION_VCPUS },
            sessionCreateOptions,
            timeout: SESSION_TIMEOUT_MS
          }
        : {
            resources: { vcpus: SESSION_VCPUS },
            sessionCreateOptions,
            source: { snapshotId: handoff.snapshotId, type: "snapshot" },
            timeout: SESSION_TIMEOUT_MS
          }
    );
  },
  async bootstrap() {},
  async onSession({ use }) {
    const sandbox = await use();
    const script = isWorkspaceDriveEnabled()
      ? workspaceDriveInitializationScript()
      : workspaceCheckoutRefreshScript();
    const result = await sandbox.run({
      command: `bash -lc ${JSON.stringify(script)}`
    });
    if (result.exitCode !== 0) {
      throw new Error(result.stderr || "Could not initialize workspace.");
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
