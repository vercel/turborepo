import * as path from "node:path";
import { detectJsRepository } from "../js/dist/index.js";

describe("detect", () => {
  it("detects a repo", async () => {
    const repo = await detectJsRepository();
    console.log(repo);
    const expectedRoot = path.resolve(__dirname, "../../..");
    expect(repo.root).toBe(expectedRoot);
  });

  it("enumerates workspaces", async () => {
    const repo = await detectJsRepository();
    const workspaces = await repo.workspaces();
    expect(workspaces.length).not.toBe(0);
  });

  // TODO: proper tests on real fixtures
});
