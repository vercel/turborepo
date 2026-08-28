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
 * sandbox. New sessions update the checkout to current main, but do not
 * re-check tools or install dependencies before giving control to the agent.
 */

/** Matches fx workspaces, and leaves room for a `cargo build`. */
const SESSION_TIMEOUT_MS = 45 * 60 * 1000;
/** `.cargo/config.toml` builds with `-Zthreads=8`. */
const SESSION_VCPUS = 8;
const WORKSPACE_CHECKOUT_PATH = "/factory/turborepo";
const WORKSPACE_DRIVE_INITIALIZED_PATH = `${WORKSPACE_DRIVE_MOUNT_PATH}/.factory-initialized`;

function workspaceCheckoutRefreshScript(): string {
  return `git -C ${WORKSPACE_CHECKOUT_PATH} fetch --depth=1 --force origin main
git -C ${WORKSPACE_CHECKOUT_PATH} reset --hard FETCH_HEAD`;
}

function workspaceInitializationScript(drivesEnabled: boolean): string {
  const refresh = workspaceCheckoutRefreshScript();
  if (!drivesEnabled) return `set -eu
${refresh}`;
  return `set -eu
refresh_checkout=0
if [ ! -f ${WORKSPACE_DRIVE_INITIALIZED_PATH} ]; then
  refresh_checkout=1
fi
${workspaceDriveInitializationScript()}
if [ "$refresh_checkout" -eq 1 ]; then
  ${refresh}
fi`;
}

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
    const result = await sandbox.run({
      command: `bash -lc ${JSON.stringify(
        workspaceInitializationScript(isWorkspaceDriveEnabled())
      )}`
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
