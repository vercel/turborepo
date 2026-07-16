import assert from "node:assert/strict";
import { describe, it, mock } from "node:test";
import { publishWithRetries } from "./npm";

describe("publishWithRetries", () => {
  it("retries npm provenance failures", async () => {
    let attempts = 0;
    const spawn = mock.fn(
      (_command: string, _args: Array<string>, _options: object) => {
        attempts += 1;
        return {
          status: attempts < 3 ? 1 : 0,
          stdout: "",
          stderr: attempts < 3 ? "TLOG_CREATE_ENTRY_ERROR" : "published"
        };
      }
    );
    const wait = mock.fn((_milliseconds: number) => Promise.resolve());

    await publishWithRetries({
      packageName: "turbo@1.0.0",
      tarball: "/tmp/turbo.tgz",
      npmTag: "latest",
      dependencies: { spawn, wait }
    });

    assert.equal(spawn.mock.callCount(), 3);
    assert.deepEqual(
      wait.mock.calls.map(({ arguments: args }) => args[0]),
      [10_000, 20_000]
    );
  });

  it("does not retry unrelated failures", async () => {
    const spawn = mock.fn(
      (_command: string, _args: Array<string>, _options: object) => ({
        status: 1,
        stdout: "",
        stderr: "npm authentication failed"
      })
    );
    const wait = mock.fn((_milliseconds: number) => Promise.resolve());

    await assert.rejects(
      publishWithRetries({
        packageName: "turbo@1.0.0",
        tarball: "/tmp/turbo.tgz",
        npmTag: "latest",
        dependencies: { spawn, wait }
      }),
      /npm publish failed/
    );
    assert.equal(spawn.mock.callCount(), 1);
    assert.equal(wait.mock.callCount(), 0);
  });
});
