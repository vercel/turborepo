import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, it, mock } from "node:test";
import { publishWithRetries } from "./npm";

const tempDirectories: Array<string> = [];

afterEach(async () => {
  await Promise.all(
    tempDirectories
      .splice(0)
      .map((directory) => rm(directory, { recursive: true, force: true }))
  );
});

async function createTarball() {
  const directory = await mkdtemp(path.join(tmpdir(), "turbo-releaser-npm-"));
  tempDirectories.push(directory);
  const contents = new TextEncoder().encode("release artifact");
  const tarball = path.join(directory, "package.tgz");
  await writeFile(tarball, contents);
  return {
    tarball,
    integrity: `sha512-${createHash("sha512").update(contents).digest("base64")}`
  };
}

const missing = {
  status: 1,
  stdout: "",
  stderr: "npm error code E404"
};

function registryMetadata(
  integrity: string,
  tagVersion = "1.0.0",
  npmTag = "latest"
) {
  return JSON.stringify({
    "dist.integrity": integrity,
    "dist.attestations": {
      provenance: { predicateType: "https://slsa.dev/provenance/v1" }
    },
    [`dist-tags.${npmTag}`]: tagVersion
  });
}

describe("publishWithRetries", () => {
  it("publishes a missing package", async () => {
    const { tarball } = await createTarball();
    const spawn = mock.fn(
      (_command: string, args: Array<string>, _options: object) =>
        args[0] === "view"
          ? missing
          : { status: 0, stdout: "published", stderr: "" }
    );

    await publishWithRetries({
      packageName: "@turbo/example",
      version: "1.0.0",
      tarball,
      npmTag: "latest",
      dependencies: { spawn, wait: async () => undefined }
    });

    assert.equal(
      spawn.mock.calls.filter(({ arguments: args }) => args[1][0] === "publish")
        .length,
      1
    );
  });

  it("skips an existing package with matching integrity", async () => {
    const { tarball, integrity } = await createTarball();
    const mismatchedIntegrity = `sha512-${createHash("sha512").update("different").digest("base64")}`;
    const spawn = mock.fn(
      (_command: string, _args: Array<string>, _options: object) => ({
        status: 0,
        stdout: registryMetadata(`${mismatchedIntegrity} ${integrity}`),
        stderr: ""
      })
    );

    await publishWithRetries({
      packageName: "turbo",
      version: "1.0.0",
      tarball,
      npmTag: "latest",
      dependencies: { spawn, wait: async () => undefined }
    });

    assert.equal(spawn.mock.callCount(), 1);
  });

  it("fails closed when the registry lookup fails", async () => {
    const { tarball } = await createTarball();
    const spawn = mock.fn(
      (_command: string, _args: Array<string>, _options: object) => ({
        status: 1,
        stdout: "",
        stderr: "npm error code E401"
      })
    );

    await assert.rejects(
      publishWithRetries({
        packageName: "turbo",
        version: "1.0.0",
        tarball,
        npmTag: "latest",
        dependencies: { spawn, wait: async () => undefined }
      }),
      /Unable to check whether turbo@1\.0\.0 exists/
    );
    assert.equal(spawn.mock.callCount(), 1);
  });

  it("accepts a package committed before npm reported failure", async () => {
    const { tarball, integrity } = await createTarball();
    let lookupCount = 0;
    const spawn = mock.fn(
      (_command: string, args: Array<string>, _options: object) => {
        if (args[0] === "publish") {
          return { status: 1, stdout: "", stderr: "socket closed" };
        }
        lookupCount += 1;
        return lookupCount === 1
          ? missing
          : {
              status: 0,
              stdout: registryMetadata(integrity),
              stderr: ""
            };
      }
    );

    await publishWithRetries({
      packageName: "turbo",
      version: "1.0.0",
      tarball,
      npmTag: "latest",
      dependencies: { spawn, wait: async () => undefined }
    });

    assert.equal(
      spawn.mock.calls.filter(({ arguments: args }) => args[1][0] === "publish")
        .length,
      1
    );
  });

  it("rejects an existing package with different contents", async () => {
    const { tarball } = await createTarball();
    const integrity = `sha512-${createHash("sha512").update("different").digest("base64")}`;
    const spawn = mock.fn(
      (_command: string, _args: Array<string>, _options: object) => ({
        status: 0,
        stdout: registryMetadata(integrity),
        stderr: ""
      })
    );

    await assert.rejects(
      publishWithRetries({
        packageName: "turbo",
        version: "1.0.0",
        tarball,
        npmTag: "latest",
        dependencies: { spawn, wait: async () => undefined }
      }),
      /different contents/
    );
    assert.equal(spawn.mock.callCount(), 1);
  });

  it("rejects an existing package when the requested tag points elsewhere", async () => {
    const { tarball, integrity } = await createTarball();
    const spawn = mock.fn(
      (_command: string, _args: Array<string>, _options: object) => ({
        status: 0,
        stdout: registryMetadata(integrity, "0.9.0"),
        stderr: ""
      })
    );

    await assert.rejects(
      publishWithRetries({
        packageName: "turbo",
        version: "1.0.0",
        tarball,
        npmTag: "latest",
        dependencies: { spawn, wait: async () => undefined }
      }),
      /npm tag latest points to 0\.9\.0/
    );
  });

  it("retries transient lookups while reconciling a failed publish", async () => {
    const { tarball, integrity } = await createTarball();
    let lookupCount = 0;
    const spawn = mock.fn(
      (_command: string, args: Array<string>, _options: object) => {
        if (args[0] === "publish") {
          return { status: 1, stdout: "", stderr: "socket closed" };
        }
        lookupCount += 1;
        if (lookupCount === 1) {
          return missing;
        }
        if (lookupCount === 2) {
          return { status: 1, stdout: "", stderr: "npm error code E500" };
        }
        return {
          status: 0,
          stdout: registryMetadata(integrity),
          stderr: ""
        };
      }
    );
    const wait = mock.fn((_milliseconds: number) => Promise.resolve());

    await publishWithRetries({
      packageName: "turbo",
      version: "1.0.0",
      tarball,
      npmTag: "latest",
      dependencies: { spawn, wait }
    });

    assert.deepEqual(
      wait.mock.calls.map(({ arguments: args }) => args[0]),
      [2000]
    );
  });

  it("retries provenance failures when the package is still missing", async () => {
    const { tarball } = await createTarball();
    let publishAttempts = 0;
    const spawn = mock.fn(
      (_command: string, args: Array<string>, _options: object) => {
        if (args[0] === "view") {
          return missing;
        }
        publishAttempts += 1;
        return {
          status: publishAttempts < 3 ? 1 : 0,
          stdout: "",
          stderr: publishAttempts < 3 ? "TLOG_CREATE_ENTRY_ERROR" : "published"
        };
      }
    );
    const wait = mock.fn((_milliseconds: number) => Promise.resolve());

    await publishWithRetries({
      packageName: "turbo",
      version: "1.0.0",
      tarball,
      npmTag: "latest",
      dependencies: { spawn, wait }
    });

    assert.equal(publishAttempts, 3);
    assert.deepEqual(
      wait.mock.calls.map(({ arguments: args }) => args[0]),
      [2000, 4000, 10_000, 2000, 4000, 20_000]
    );
  });
});
