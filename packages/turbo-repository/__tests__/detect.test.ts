import * as path from "node:path";
import { Repository } from "../js/dist/index.js";

describe("detect", () => {
  it("detects a repo", async () => {
    const repo = await Repository.discover();
    console.log(repo);
    const expectedRoot = path.resolve(__dirname, "../../..");
    expect(repo.root).toBe(expectedRoot);
  });

  it("enumerates workspaces", async () => {
    const repo = await Repository.discover();
    const workspaces = await repo.workspaces();
    expect(workspaces.length).not.toBe(0);
  });

  // TODO: proper tests on real fixtures
});
