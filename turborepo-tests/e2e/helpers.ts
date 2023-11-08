import path from "path";
import * as fs from "fs";
import execa from "execa";
import { Readable } from "stream";
// @ts-ignore-next-line
import tar from "tar";
// @ts-ignore-next-line
import { ZstdCodec } from "zstd-codec";
import { Monorepo } from "./monorepo";
import type { DryRun, PackageManager } from "./types";

export const matchTask =
  <T, V>(predicate: (dryRun: DryRun, val: T) => V) =>
  (dryRun: DryRun) =>
  (val: T): V => {
    return predicate(dryRun, val);
  };

const includesTaskIdPredicate = (dryRun: DryRun, task: string): boolean => {
  for (const entry of dryRun.tasks) {
    if (entry.taskId === task) {
      return true;
    }
  }
  return false;
};

export const includesTaskId = matchTask(includesTaskIdPredicate);

export const taskHashPredicate = (dryRun: DryRun, taskId: string): string => {
  for (const entry of dryRun.tasks) {
    if (entry.taskId === taskId) {
      return entry.hash;
    }
  }
  throw new Error(`missing task with id ${taskId}`);
};

// getLockfileForPackageManager returns the name of the lockfile for the given package manager
export function getLockfileForPackageManager(ws: PackageManager) {
  switch (ws) {
    case "yarn":
      return "yarn.lock";
    case "pnpm":
      return "pnpm-lock.yaml";
    case "pnpm6":
      return "pnpm-lock.yaml";
    case "npm":
      return "package-lock.json";
    case "berry":
      return "yarn.lock";
    default:
      throw new Error(`Unknown package manager: ${ws}`);
  }
}

export function getImmutableInstallForPackageManager(
  ws: PackageManager
): string[] {
  switch (ws) {
    case "yarn":
      return ["install", "--frozen-lockfile"];
    case "pnpm":
      return ["install", "--frozen-lockfile"];
    case "pnpm6":
      return ["install", "--frozen-lockfile"];
    case "npm":
      return ["ci"];
    case "berry":
      return ["install", "--immutable"];
    default:
      throw new Error(`Unknown package manager: ${ws}`);
  }
}
export function getCommandOutputAsArray(
  results: execa.ExecaSyncReturnValue<string>
): string[] {
  return (results.stdout + "\n" + results.stderr)
    .split("\n")
    .map((line) => line.replace("\r", ""));
}

export function getHashFromOutput(lines: string[], taskId: string): string {
  const normalizedTaskId = taskId.replace("#", ":");
  const line = lines.find((l) => l.startsWith(normalizedTaskId)) as string;
  const splitMessage = line.split(" ");
  const hash = splitMessage[splitMessage.length - 1];
  return hash;
}

export function getCacheItemForHash(repo: Monorepo, hash: string): string {
  return path.join(
    repo.subdir ? repo.subdir : ".",
    "node_modules",
    ".cache",
    "turbo",
    `${hash}.tar.zst`
  );
}

export function getCachedLogFilePathForTask(
  repo: Monorepo,
  pathToPackage: string,
  taskName: string
): string {
  return path.join(
    repo.subdir ? repo.subdir : "",
    pathToPackage,
    ".turbo",
    `turbo-${taskName}.log`
  );
}

function createDecoder() {
  return new Promise((resolve) => {
    // @ts-ignore-next-line
    ZstdCodec.run((zstd) => resolve(new zstd.Streaming()));
  });
}

// @ts-ignore-next-line
export async function extractZst(zst, dest: string) {
  let decoder = await createDecoder();
  const fileBuffer = fs.readFileSync(zst);
  const data = new Uint8Array(
    fileBuffer.buffer.slice(
      fileBuffer.byteOffset,
      fileBuffer.byteOffset + fileBuffer.byteLength
    )
  );
  // @ts-ignore-next-line
  const decompressed = decoder.decompress(data);
  const stream = Readable.from(Buffer.from(decompressed));
  const output = stream.pipe(
    tar.x({
      cwd: dest,
    })
  );

  return new Promise((resolve) => {
    output.on("finish", resolve);
  });
}
