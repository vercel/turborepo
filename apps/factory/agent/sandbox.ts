import { defineSandbox, type SandboxSession } from "eve/sandbox";
import { vercel } from "eve/sandbox/vercel";

import {
  FACTORY_IMAGE_SPEC,
  factoryImageFingerprint,
  runFactoryImagePhases
} from "./lib/factory-image";
import {
  readFactoryImageHandoff,
  writeFactoryImageHandoff
} from "./lib/factory-image-handoff";
import { readFactoryImagePointer } from "./lib/factory-image-registry";
import { fetchMainCommit } from "./lib/github";

/**
 * Sandbox for every Eve run in this app.
 *
 * The template is the factory image: a Turborepo checkout plus the whole
 * `cargo build` and `pnpm test` toolchain, provisioned from the phases in
 * `lib/factory-image.ts`. When the merge webhook has already published a
 * snapshot for this toolchain the template boots from it and the phases
 * only fast-forward; otherwise they install everything from scratch.
 *
 * `revalidationKey` rotates on two inputs: the toolchain fingerprint (a
 * pinned version changed) and the published snapshot (a merge produced a
 * newer image). Eve freezes that key at build time, so the template is
 * stable within a deployment and each session fast-forwards its checkout
 * to the current `main`.
 */

/** Matches the Harness path, and leaves room for a `cargo build`. */
const SESSION_TIMEOUT_MS = 45 * 60 * 1000;
/** `.cargo/config.toml` builds with `-Zthreads=8`. */
const SESSION_VCPUS = 8;

async function runOrThrow(sandbox: SandboxSession, command: string) {
  const result = await sandbox.run({ command });
  if (result.exitCode !== 0) {
    throw new Error(`${command} failed: ${result.stderr}`);
  }
}

/**
 * Commit the template checks out. The freshest revision wins, so the
 * template starts as close to `main` as the build environment allows.
 */
async function resolveTemplateRevision(): Promise<string> {
  const handoff = readFactoryImageHandoff(factoryImageFingerprint());
  try {
    return await fetchMainCommit();
  } catch (error) {
    if (handoff !== null) {
      console.warn(
        "Could not resolve main; using the published image commit.",
        error
      );
      return handoff.commit;
    }
    console.warn("Could not resolve main; checking out the branch.", error);
    return "main";
  }
}

export default defineSandbox({
  backend: () => {
    const handoff = readFactoryImageHandoff(factoryImageFingerprint());
    const resources = { vcpus: SESSION_VCPUS };
    return handoff === null
      ? vercel({ resources, timeout: SESSION_TIMEOUT_MS })
      : vercel({
          resources,
          source: { snapshotId: handoff.snapshotId, type: "snapshot" },
          timeout: SESSION_TIMEOUT_MS
        });
  },
  revalidationKey: async () => {
    const fingerprint = factoryImageFingerprint();
    const pointer = await readFactoryImagePointer();
    const base =
      pointer !== null && pointer.fingerprint === fingerprint ? pointer : null;
    writeFactoryImageHandoff({
      commit: base?.commit,
      fingerprint,
      snapshotId: base?.snapshotId
    });
    return `factory-image:${fingerprint}:${base?.snapshotId ?? "none"}`;
  },
  async bootstrap({ use }) {
    const sandbox = await use();
    const revision = await resolveTemplateRevision();
    await runFactoryImagePhases(
      { run: (command) => sandbox.run({ command }) },
      { revision },
      (phase) => console.log(`factory image: ${phase.title}`)
    );
  },
  async onSession({ use }) {
    const sandbox = await use();
    const repository = FACTORY_IMAGE_SPEC.checkoutPath;
    // The template already carries the toolchain, node_modules, and the
    // Cargo registry, so catching up to main is a shallow fetch plus an
    // incremental install.
    await runOrThrow(
      sandbox,
      `git -C ${repository} fetch --depth=1 --force origin main && git -C ${repository} reset --hard FETCH_HEAD && git -C ${repository} clean -ffd`
    );
    await runOrThrow(
      sandbox,
      `cd ${repository} && pnpm install --frozen-lockfile`
    );
  }
});
