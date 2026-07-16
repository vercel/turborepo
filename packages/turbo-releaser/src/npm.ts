import { spawnSync } from "node:child_process";
import { setTimeout } from "node:timers/promises";

const PROVENANCE_ERROR = /TLOG_CREATE_ENTRY_ERROR|error creating tlog entry/;
const NPM_OUTPUT_BUFFER_BYTES = 50 * 1024 * 1024;

interface PublishDependencies {
  spawn: (
    command: string,
    args: Array<string>,
    options: { encoding: "utf8"; maxBuffer: number }
  ) => { status: number | null; stdout: string | null; stderr: string | null };
  wait: (milliseconds: number) => Promise<unknown>;
}

export async function publishWithRetries({
  packageName,
  tarball,
  npmTag,
  accessPublic = false,
  dependencies = {
    spawn: (command, args, options) => spawnSync(command, args, options),
    wait: setTimeout
  }
}: {
  packageName: string;
  tarball: string;
  npmTag: string;
  accessPublic?: boolean;
  dependencies?: PublishDependencies;
}) {
  const maxAttempts = 3;

  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    console.log(
      `Publishing ${packageName} (attempt ${attempt}/${maxAttempts})`
    );
    const args = ["publish", "-ddd", "--tag", npmTag, tarball];
    if (accessPublic) {
      args.push("--access", "public");
    }
    const result = dependencies.spawn("npm", args, {
      encoding: "utf8",
      maxBuffer: NPM_OUTPUT_BUFFER_BYTES
    });
    process.stdout.write(result.stdout ?? "");
    process.stderr.write(result.stderr ?? "");

    if (result.status === 0) {
      return;
    }

    const output = `${result.stdout ?? ""}\n${result.stderr ?? ""}`;
    if (!PROVENANCE_ERROR.test(output) || attempt === maxAttempts) {
      throw new Error(
        `npm publish failed for ${packageName} with status ${result.status ?? "unknown"}`
      );
    }

    const delaySeconds = attempt * 10;
    console.log(
      `Retrying ${packageName} after npm provenance tlog failure in ${delaySeconds} seconds...`
    );
    await dependencies.wait(delaySeconds * 1000);
  }
}
