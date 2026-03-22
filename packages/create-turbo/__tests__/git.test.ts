import path from "node:path";
import childProcess from "node:child_process";
import type { SpawnSyncReturns } from "node:child_process";
import fs from "node:fs";
import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect, jest } from "@jest/globals";
import {
  DEFAULT_IGNORE,
  tryGitInit,
  removeGitDirectory
} from "../src/utils/git";

function spawnResult(status: number): SpawnSyncReturns<Buffer> {
  return {
    pid: 1,
    output: [],
    stdout: Buffer.from(""),
    stderr: Buffer.from(""),
    status,
    signal: null
  };
}

const SUCCESS = spawnResult(0);
const FAILURE = spawnResult(1);

describe("git", () => {
  // just to make sure this doesn't get lost
  it("default .gitignore includes .turbo", async () => {
    expect(DEFAULT_IGNORE).toContain(".turbo");
  });

  describe("tryGitInit", () => {
    const { useFixture } = setupTestFixtures({
      directory: path.join(__dirname, "../"),
      options: { emptyFixture: true }
    });

    it("inits a repo with a single commit", async () => {
      const { root } = useFixture({ fixture: `git` });
      const mockSpawnSync = jest
        .spyOn(childProcess, "spawnSync")
        .mockReturnValueOnce(FAILURE) // git rev-parse (not in git repo)
        .mockReturnValueOnce(FAILURE) // hg --cwd . root (not in hg repo)
        .mockReturnValue(SUCCESS);

      const result = tryGitInit(root);
      expect(result).toBe(true);

      const expectedCalls: Array<[string, Array<string>]> = [
        ["git", ["rev-parse", "--is-inside-work-tree"]],
        ["hg", ["--cwd", ".", "root"]],
        ["git", ["init"]],
        ["git", ["checkout", "-b", "main"]],
        ["git", ["add", "-A"]],
        ["git", ["commit", "-m", "Initial commit from create-turbo"]]
      ];
      expect(mockSpawnSync).toHaveBeenCalledTimes(expectedCalls.length);
      for (const [cmd, args] of expectedCalls) {
        expect(mockSpawnSync).toHaveBeenCalledWith(cmd, args, {
          stdio: "ignore",
          cwd: root
        });
      }
      mockSpawnSync.mockRestore();
    });

    it("creates exactly one commit with all changes", async () => {
      const { root } = useFixture({ fixture: `git` });
      const commitCalls: Array<Array<string>> = [];
      const mockSpawnSync = jest
        .spyOn(childProcess, "spawnSync")
        .mockImplementation((command, args) => {
          const cmd = String(command);
          const argList = (args ?? []) as Array<string>;
          if (cmd === "git" && argList[0] === "rev-parse") {
            return FAILURE;
          }
          if (cmd === "hg") {
            return FAILURE;
          }
          if (cmd === "git" && argList[0] === "commit") {
            commitCalls.push(argList);
          }
          return SUCCESS;
        });

      tryGitInit(root);

      expect(commitCalls).toHaveLength(1);
      expect(commitCalls[0]).toEqual([
        "commit",
        "-m",
        "Initial commit from create-turbo"
      ]);
      mockSpawnSync.mockRestore();
    });

    it("runs all git commands in the project root directory", async () => {
      const { root } = useFixture({ fixture: `git` });
      const cwdValues: Array<string | undefined> = [];
      const mockSpawnSync = jest
        .spyOn(childProcess, "spawnSync")
        .mockImplementation((command, args, options) => {
          const opts = options as { cwd?: string };
          cwdValues.push(opts?.cwd);
          const cmd = String(command);
          const argList = (args ?? []) as Array<string>;
          if (cmd === "git" && argList[0] === "rev-parse") {
            return FAILURE;
          }
          if (cmd === "hg") {
            return FAILURE;
          }
          return SUCCESS;
        });

      tryGitInit(root);

      expect(cwdValues.every((cwd) => cwd === root)).toBe(true);
      mockSpawnSync.mockRestore();
    });

    it("skips init if already in a git repo", async () => {
      const { root } = useFixture({
        fixture: `git`
      });
      const mockSpawnSync = jest
        .spyOn(childProcess, "spawnSync")
        .mockReturnValueOnce(SUCCESS) // git rev-parse succeeds
        .mockReturnValue(SUCCESS);

      const result = tryGitInit(root);
      expect(result).toBe(false);

      expect(mockSpawnSync).toHaveBeenCalledTimes(1);
      expect(mockSpawnSync).toHaveBeenCalledWith(
        "git",
        ["rev-parse", "--is-inside-work-tree"],
        { stdio: "ignore", cwd: root }
      );
      mockSpawnSync.mockRestore();
    });

    it("returns false on unexpected error during init", async () => {
      const { root } = useFixture({ fixture: `git` });
      const mockSpawnSync = jest
        .spyOn(childProcess, "spawnSync")
        .mockReturnValueOnce(FAILURE) // not in git repo
        .mockReturnValueOnce(FAILURE) // not in hg repo
        .mockReturnValueOnce(FAILURE); // git init fails

      const result = tryGitInit(root);
      expect(result).toBe(false);

      const expectedCalls: Array<[string, Array<string>]> = [
        ["git", ["rev-parse", "--is-inside-work-tree"]],
        ["hg", ["--cwd", ".", "root"]],
        ["git", ["init"]]
      ];

      expect(mockSpawnSync).toHaveBeenCalledTimes(expectedCalls.length);
      for (const [cmd, args] of expectedCalls) {
        expect(mockSpawnSync).toHaveBeenCalledWith(cmd, args, {
          stdio: "ignore",
          cwd: root
        });
      }
      mockSpawnSync.mockRestore();
    });

    it("cleans up .git directory on failure after init", async () => {
      const { root } = useFixture({ fixture: `git` });
      const mockRmSync = jest.spyOn(fs, "rmSync").mockImplementation(() => {});
      const mockSpawnSync = jest
        .spyOn(childProcess, "spawnSync")
        .mockReturnValueOnce(FAILURE) // not in git repo
        .mockReturnValueOnce(FAILURE) // not in hg repo
        .mockReturnValueOnce(SUCCESS) // git init succeeds
        .mockReturnValueOnce(FAILURE); // git checkout -b main fails

      const result = tryGitInit(root);
      expect(result).toBe(false);

      expect(mockRmSync).toHaveBeenCalledWith(path.join(root, ".git"), {
        recursive: true,
        force: true
      });
      mockSpawnSync.mockRestore();
      mockRmSync.mockRestore();
    });

    it("cleans up .git directory when user has no git config (commit fails)", async () => {
      const { root } = useFixture({ fixture: `git` });
      const mockRmSync = jest.spyOn(fs, "rmSync").mockImplementation(() => {});
      const mockSpawnSync = jest
        .spyOn(childProcess, "spawnSync")
        .mockReturnValueOnce(FAILURE) // not in git repo
        .mockReturnValueOnce(FAILURE) // not in hg repo
        .mockReturnValueOnce(SUCCESS) // git init
        .mockReturnValueOnce(SUCCESS) // git checkout -b main
        .mockReturnValueOnce(SUCCESS) // git add -A
        .mockReturnValueOnce(FAILURE); // git commit fails

      const result = tryGitInit(root);
      expect(result).toBe(false);

      expect(mockRmSync).toHaveBeenCalledWith(path.join(root, ".git"), {
        recursive: true,
        force: true
      });
      mockSpawnSync.mockRestore();
      mockRmSync.mockRestore();
    });

    it("skips init if already in a mercurial repo", async () => {
      const { root } = useFixture({
        fixture: `git`
      });
      const mockSpawnSync = jest
        .spyOn(childProcess, "spawnSync")
        .mockReturnValueOnce(FAILURE) // not in git repo
        .mockReturnValueOnce(SUCCESS) // hg repo check succeeds
        .mockReturnValue(SUCCESS);

      const result = tryGitInit(root);
      expect(result).toBe(false);

      expect(mockSpawnSync).toHaveBeenCalledTimes(2);
      expect(mockSpawnSync).toHaveBeenCalledWith(
        "git",
        ["rev-parse", "--is-inside-work-tree"],
        { stdio: "ignore", cwd: root }
      );
      expect(mockSpawnSync).toHaveBeenCalledWith("hg", ["--cwd", ".", "root"], {
        stdio: "ignore",
        cwd: root
      });
      mockSpawnSync.mockRestore();
    });

    it("rejects directory paths with shell metacharacters", async () => {
      const dangerousPaths = [
        "/tmp/$(whoami)",
        "/tmp/`id`",
        "/tmp/foo;rm -rf /",
        "/tmp/foo|cat /etc/passwd",
        "/tmp/foo&bg"
      ];

      for (const unsafePath of dangerousPaths) {
        expect(() => tryGitInit(unsafePath)).toThrow(
          "Directory path contains potentially unsafe characters"
        );
      }
    });
  });

  describe("removeGitDirectory", () => {
    const { useFixture } = setupTestFixtures({
      directory: path.join(__dirname, "../"),
      options: { emptyFixture: true }
    });

    it("attempts to remove .git directory", async () => {
      const { root } = useFixture({ fixture: `remove-git` });
      const mockRmSync = jest.spyOn(fs, "rmSync").mockImplementation(() => {});

      const result = removeGitDirectory(root);
      expect(result).toBe(true);

      expect(mockRmSync).toHaveBeenCalledWith(path.join(root, ".git"), {
        recursive: true,
        force: true
      });
      mockRmSync.mockRestore();
    });

    it("returns false on error", async () => {
      const { root } = useFixture({ fixture: `remove-git-error` });
      const mockRmSync = jest.spyOn(fs, "rmSync").mockImplementation(() => {
        throw new Error("Permission denied");
      });

      const result = removeGitDirectory(root);
      expect(result).toBe(false);

      expect(mockRmSync).toHaveBeenCalledWith(path.join(root, ".git"), {
        recursive: true,
        force: true
      });
      mockRmSync.mockRestore();
    });
  });
});
