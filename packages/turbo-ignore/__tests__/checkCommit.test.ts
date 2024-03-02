import child_process from "node:child_process";
import { mockEnv } from "@turbo/test-utils";
import { checkCommit } from "../src/checkCommit";

describe("checkCommit()", () => {
  describe("on Vercel", () => {
    mockEnv();

    describe("for all workspaces", () => {
      it("results in continue when no special commit messages are found", async () => {
        process.env.VERCEL = "1";
        process.env.VERCEL_GIT_COMMIT_MESSAGE = "fixing a test";
        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "continue",
          scope: "global",
          reason: "No deploy or skip string found in commit message.",
        });
      });

      it("results in conflict when deploy and skip commit messages are found", async () => {
        process.env.VERCEL = "1";
        process.env.VERCEL_GIT_COMMIT_MESSAGE =
          "deploying [vercel deploy] and skipping [vercel skip]";
        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "conflict",
          scope: "global",
          reason:
            "Conflicting commit messages found: [vercel deploy] and [vercel skip]",
        });
      });

      it("results in deploy when deploy commit message is found", async () => {
        process.env.VERCEL = "1";
        process.env.VERCEL_GIT_COMMIT_MESSAGE = "deploying [vercel deploy]";
        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "deploy",
          scope: "global",
          reason: "Found commit message: [vercel deploy]",
        });
      });

      it("results in skip when skip commit message is found", async () => {
        process.env.VERCEL = "1";
        process.env.VERCEL_GIT_COMMIT_MESSAGE = "skip deployment [vercel skip]";
        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "skip",
          scope: "global",
          reason: "Found commit message: [vercel skip]",
        });
      });
    });

    describe("for specific workspaces", () => {
      it("results in continue when no special commit messages are found", async () => {
        process.env.VERCEL = "1";
        process.env.VERCEL_GIT_COMMIT_MESSAGE =
          "fixing a test in test-workspace";
        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "continue",
          scope: "global",
          reason: "No deploy or skip string found in commit message.",
        });
      });

      it("results in conflict when deploy and skip commit messages are found", async () => {
        process.env.VERCEL = "1";
        process.env.VERCEL_GIT_COMMIT_MESSAGE =
          "deploying [vercel deploy test-workspace] and skipping [vercel skip test-workspace]";
        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "conflict",
          scope: "workspace",
          reason:
            "Conflicting commit messages found: [vercel deploy test-workspace] and [vercel skip test-workspace]",
        });
      });

      it("results in deploy when deploy commit message is found", async () => {
        process.env.VERCEL = "1";
        process.env.VERCEL_GIT_COMMIT_MESSAGE =
          "deploying [vercel deploy test-workspace]";
        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "deploy",
          scope: "workspace",
          reason: "Found commit message: [vercel deploy test-workspace]",
        });
      });

      it("results in skip when skip commit message is found", async () => {
        process.env.VERCEL = "1";
        process.env.VERCEL_GIT_COMMIT_MESSAGE =
          "skip deployment [vercel skip test-workspace]";
        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "skip",
          scope: "workspace",
          reason: "Found commit message: [vercel skip test-workspace]",
        });
      });

      it("results in deploy when deploy only is found", async () => {
        process.env.VERCEL = "1";
        process.env.VERCEL_GIT_COMMIT_MESSAGE =
          "deploying [vercel only test-workspace]";
        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "deploy",
          scope: "workspace",
          reason: "Found commit message: [vercel only test-workspace]",
        });
      });

      it("results in skip when deploy not match workspace", async () => {
        process.env.VERCEL = "1";
        process.env.VERCEL_GIT_COMMIT_MESSAGE =
          "deploying [vercel only test-workspace]";
        expect(checkCommit({ workspace: "test-workspace2" })).toEqual({
          result: "skip",
          scope: "workspace",
          reason: "Found commit message: [vercel only test-workspace]",
        });
      });
    });
  });
  describe("Not on Vercel", () => {
    describe("for all workspaces", () => {
      it("results in continue when no special commit messages are found", async () => {
        const commitBody = "fixing a test";
        const mockExecSync = jest
          .spyOn(child_process, "execSync")
          .mockImplementation((_) => commitBody);

        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "continue",
          scope: "global",
          reason: "No deploy or skip string found in commit message.",
        });
        expect(mockExecSync).toHaveBeenCalledWith("git show -s --format=%B");
        mockExecSync.mockRestore();
      });

      it("results in conflict when deploy and skip commit messages are found", async () => {
        const commitBody =
          "deploying [vercel deploy] and skipping [vercel skip]";
        const mockExecSync = jest
          .spyOn(child_process, "execSync")
          .mockImplementation((_) => commitBody);

        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "conflict",
          scope: "global",
          reason:
            "Conflicting commit messages found: [vercel deploy] and [vercel skip]",
        });
        expect(mockExecSync).toHaveBeenCalledWith("git show -s --format=%B");
        mockExecSync.mockRestore();
      });

      it("results in deploy when deploy commit message is found", async () => {
        const commitBody = "deploying [vercel deploy]";
        const mockExecSync = jest
          .spyOn(child_process, "execSync")
          .mockImplementation((_) => commitBody);

        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "deploy",
          scope: "global",
          reason: "Found commit message: [vercel deploy]",
        });
        expect(mockExecSync).toHaveBeenCalledWith("git show -s --format=%B");
        mockExecSync.mockRestore();
      });

      it("results in skip when skip commit message is found", async () => {
        const commitBody = "skip deployment [vercel skip]";
        const mockExecSync = jest
          .spyOn(child_process, "execSync")
          .mockImplementation((_) => commitBody);

        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "skip",
          scope: "global",
          reason: "Found commit message: [vercel skip]",
        });
        expect(mockExecSync).toHaveBeenCalledWith("git show -s --format=%B");
        mockExecSync.mockRestore();
      });
    });

    describe("for specific workspaces", () => {
      it("results in continue when no special commit messages are found", async () => {
        const commitBody = "fixing a test in test-workspace";
        const mockExecSync = jest
          .spyOn(child_process, "execSync")
          .mockImplementation((_) => commitBody);

        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "continue",
          scope: "global",
          reason: "No deploy or skip string found in commit message.",
        });
        expect(mockExecSync).toHaveBeenCalledWith("git show -s --format=%B");
        mockExecSync.mockRestore();
      });

      it("results in conflict when deploy and skip commit messages are found", async () => {
        const commitBody =
          "deploying [vercel deploy test-workspace] and skipping [vercel skip test-workspace]";
        const mockExecSync = jest
          .spyOn(child_process, "execSync")
          .mockImplementation((_) => commitBody);

        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "conflict",
          scope: "workspace",
          reason:
            "Conflicting commit messages found: [vercel deploy test-workspace] and [vercel skip test-workspace]",
        });
        expect(mockExecSync).toHaveBeenCalledWith("git show -s --format=%B");
        mockExecSync.mockRestore();
      });

      it("results in deploy when deploy commit message is found", async () => {
        const commitBody = "deploying [vercel deploy test-workspace]";
        const mockExecSync = jest
          .spyOn(child_process, "execSync")
          .mockImplementation((_) => commitBody);

        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "deploy",
          scope: "workspace",
          reason: "Found commit message: [vercel deploy test-workspace]",
        });
        expect(mockExecSync).toHaveBeenCalledWith("git show -s --format=%B");
        mockExecSync.mockRestore();
      });

      it("results in skip when skip commit message is found", async () => {
        const commitBody = "skip deployment [vercel skip test-workspace]";
        const mockExecSync = jest
          .spyOn(child_process, "execSync")
          .mockImplementation((_) => commitBody);

        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "skip",
          scope: "workspace",
          reason: "Found commit message: [vercel skip test-workspace]",
        });
        expect(mockExecSync).toHaveBeenCalledWith("git show -s --format=%B");
        mockExecSync.mockRestore();
      });

      it("results in deploy when deploy only is found", async () => {
        const commitBody = "deploying [vercel only test-workspace]";
        const mockExecSync = jest
          .spyOn(child_process, "execSync")
          .mockImplementation((_) => commitBody);

        expect(checkCommit({ workspace: "test-workspace" })).toEqual({
          result: "deploy",
          scope: "workspace",
          reason: "Found commit message: [vercel only test-workspace]",
        });
        expect(mockExecSync).toHaveBeenCalledWith("git show -s --format=%B");
        mockExecSync.mockRestore();
      });

      it("results in skip when deploy not match workspace", async () => {
        const commitBody = "deploying [vercel only test-workspace]";
        const mockExecSync = jest
          .spyOn(child_process, "execSync")
          .mockImplementation((_) => commitBody);

        expect(checkCommit({ workspace: "test-workspace2" })).toEqual({
          result: "skip",
          scope: "workspace",
          reason: "Found commit message: [vercel only test-workspace]",
        });
        expect(mockExecSync).toHaveBeenCalledWith("git show -s --format=%B");
        mockExecSync.mockRestore();
      });
    });
  });
});
