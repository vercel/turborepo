import { spawnSync } from "node:child_process";
import { createHash, timingSafeEqual } from "node:crypto";
import { readFile } from "node:fs/promises";
import { setTimeout } from "node:timers/promises";

const PROVENANCE_ERROR = /TLOG_CREATE_ENTRY_ERROR|error creating tlog entry/;
const NOT_FOUND_ERROR = /\bcode E404\b/;
const NPM_OUTPUT_BUFFER_BYTES = 50 * 1024 * 1024;
const NPM_REGISTRY = "https://registry.npmjs.org";
const SUPPORTED_INTEGRITY_ALGORITHMS = ["sha512", "sha384", "sha256"] as const;

interface SpawnResult {
  status: number | null;
  stdout: string | null;
  stderr: string | null;
  error?: Error;
}

interface PublishDependencies {
  spawn: (
    command: string,
    args: Array<string>,
    options: { encoding: "utf8"; maxBuffer: number }
  ) => SpawnResult;
  wait: (milliseconds: number) => Promise<unknown>;
}

type RegistryPackage =
  | { status: "missing" }
  | { status: "published"; integrity: string; tagVersion: string };

const defaultDependencies: PublishDependencies = {
  spawn: (command, args, options) => spawnSync(command, args, options),
  wait: setTimeout
};

export async function publishWithRetries({
  packageName,
  version,
  tarball,
  npmTag,
  accessPublic = false,
  dependencies = defaultDependencies
}: {
  packageName: string;
  version: string;
  tarball: string;
  npmTag: string;
  accessPublic?: boolean;
  dependencies?: PublishDependencies;
}) {
  const packageSpec = `${packageName}@${version}`;
  const tarballContents = new Uint8Array(await readFile(tarball));
  const maxAttempts = 3;

  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    const existingPackage = lookupPackage(packageSpec, npmTag, dependencies);
    if (existingPackage.status === "published") {
      verifyPublishedPackage(
        packageSpec,
        version,
        tarballContents,
        npmTag,
        existingPackage
      );
      console.log(
        `${packageSpec} already exists with matching integrity. Skipping.`
      );
      return;
    }

    console.log(
      `Publishing ${packageSpec} (attempt ${attempt}/${maxAttempts})`
    );
    const args = [
      "publish",
      "-ddd",
      "--registry",
      NPM_REGISTRY,
      "--tag",
      npmTag,
      tarball
    ];
    if (accessPublic) {
      args.push("--access", "public");
    }
    const result = dependencies.spawn("npm", args, {
      encoding: "utf8",
      maxBuffer: NPM_OUTPUT_BUFFER_BYTES
    });
    process.stdout.write(result.stdout ?? "");
    process.stderr.write(result.stderr ?? "");

    if (result.status === 0 && !result.error) {
      return;
    }

    if (
      await publishedAfterFailure(
        packageSpec,
        version,
        tarballContents,
        npmTag,
        dependencies
      )
    ) {
      console.log(
        `${packageSpec} was published despite npm reporting an error.`
      );
      return;
    }

    const output = `${result.stdout ?? ""}\n${result.stderr ?? ""}`;
    if (!PROVENANCE_ERROR.test(output) || attempt === maxAttempts) {
      throw new Error(
        `npm publish failed for ${packageSpec} with status ${result.status ?? "unknown"}`,
        { cause: result.error }
      );
    }

    const delaySeconds = attempt * 10;
    console.log(
      `Retrying ${packageSpec} after npm provenance tlog failure in ${delaySeconds} seconds...`
    );
    await dependencies.wait(delaySeconds * 1000);
  }
}

function lookupPackage(
  packageSpec: string,
  npmTag: string,
  dependencies: PublishDependencies
): RegistryPackage {
  const tagField = `dist-tags.${npmTag}`;
  const result = dependencies.spawn(
    "npm",
    [
      "view",
      packageSpec,
      "dist.integrity",
      "dist.attestations",
      tagField,
      "--json",
      "--registry",
      NPM_REGISTRY
    ],
    {
      encoding: "utf8",
      maxBuffer: NPM_OUTPUT_BUFFER_BYTES
    }
  );
  if (result.status !== 0 || result.error) {
    const output = `${result.stdout ?? ""}\n${result.stderr ?? ""}`;
    if (NOT_FOUND_ERROR.test(output)) {
      return { status: "missing" };
    }
    throw new Error(
      `Unable to check whether ${packageSpec} exists on npm (status ${result.status ?? "unknown"})`,
      { cause: result.error }
    );
  }

  let metadata: unknown;
  try {
    metadata = JSON.parse(result.stdout ?? "");
  } catch (error) {
    throw new Error(`npm returned invalid metadata for ${packageSpec}`, {
      cause: error
    });
  }
  if (!metadata || typeof metadata !== "object" || Array.isArray(metadata)) {
    throw new Error(`npm returned invalid metadata for ${packageSpec}`);
  }
  const integrity = Reflect.get(metadata, "dist.integrity") as unknown;
  const attestations = Reflect.get(metadata, "dist.attestations") as unknown;
  const tagVersion = Reflect.get(metadata, tagField) as unknown;
  if (typeof integrity !== "string" || integrity.length === 0) {
    throw new Error(`npm returned no integrity metadata for ${packageSpec}`);
  }
  if (typeof tagVersion !== "string" || tagVersion.length === 0) {
    throw new Error(`npm tag ${npmTag} does not point to ${packageSpec}`);
  }
  if (
    !attestations ||
    typeof attestations !== "object" ||
    Reflect.get(attestations, "provenance") === undefined
  ) {
    throw new Error(
      `npm returned no provenance attestation for ${packageSpec}`
    );
  }
  return { status: "published", integrity, tagVersion };
}

async function publishedAfterFailure(
  packageSpec: string,
  version: string,
  tarballContents: Uint8Array,
  npmTag: string,
  dependencies: PublishDependencies
) {
  let lastLookupError: unknown;
  for (let check = 1; check <= 3; check += 1) {
    if (check > 1) {
      await dependencies.wait((check - 1) * 2000);
    }
    let existingPackage: RegistryPackage;
    try {
      existingPackage = lookupPackage(packageSpec, npmTag, dependencies);
      lastLookupError = undefined;
    } catch (error) {
      lastLookupError = error;
      continue;
    }
    if (existingPackage.status === "published") {
      verifyPublishedPackage(
        packageSpec,
        version,
        tarballContents,
        npmTag,
        existingPackage
      );
      return true;
    }
  }
  if (lastLookupError) {
    throw new Error(`Unable to reconcile failed publish for ${packageSpec}`, {
      cause: lastLookupError
    });
  }
  return false;
}

function verifyPublishedPackage(
  packageSpec: string,
  version: string,
  tarballContents: Uint8Array,
  npmTag: string,
  registryPackage: Extract<RegistryPackage, { status: "published" }>
) {
  verifyIntegrity(packageSpec, tarballContents, registryPackage.integrity);
  if (registryPackage.tagVersion !== version) {
    throw new Error(
      `${packageSpec} exists with matching integrity, but npm tag ${npmTag} points to ${registryPackage.tagVersion}.`
    );
  }
}

function verifyIntegrity(
  packageSpec: string,
  tarballContents: Uint8Array,
  registryIntegrity: string
) {
  const entries = registryIntegrity
    .trim()
    .split(/\s+/)
    .map((entry) =>
      /^(sha512|sha384|sha256)-([A-Za-z0-9+/]+={0,2})$/.exec(entry)
    )
    .filter((entry) => entry !== null);
  const algorithm = SUPPORTED_INTEGRITY_ALGORITHMS.find((candidate) =>
    entries.some((entry) => entry[1] === candidate)
  );
  if (!algorithm) {
    throw new Error(
      `npm returned unsupported integrity metadata for ${packageSpec}: ${registryIntegrity}`
    );
  }

  const actual = new Uint8Array(
    createHash(algorithm).update(tarballContents).digest()
  );
  const matches = entries
    .filter((entry) => entry[1] === algorithm)
    .some((entry) => {
      const expected = new Uint8Array(Buffer.from(entry[2], "base64"));
      return (
        expected.length === actual.length && timingSafeEqual(expected, actual)
      );
    });
  if (!matches) {
    throw new Error(
      `${packageSpec} already exists on npm with different contents. This version cannot be resumed safely.`
    );
  }
}
