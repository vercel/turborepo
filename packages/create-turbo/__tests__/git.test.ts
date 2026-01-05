import path from "node:path";
import childProcess from "node:child_process";
import fs from "node:fs";
import { setupTestFixtures } from "@turbo/test-utils";
import { describe, it, expect, jest } from "@jest/globals";
import {
  DEFAULT_IGNORE,
  GIT_REPO_COMMAND,
  HG_REPO_COMMAND,
  tryGitInit,
  removeGitDirectory,
} from "../src/utils/git";

describe("git", () => {
  // just to make sure this doesn't get lost
  it("default .gitignore includes .turbo", async () => {
    expect(DEFAULT_IGNORE).toContain(".turbo");
  });

  describe("tryGitInit", () => {
    const { useFixture } = setupTestFixtures({
      directory: path.join(__dirname, "../"),
      options: { emptyFixture: true },
    });

    it("inits a repo with a single commit", async () => {
      const { root } = useFixture({ fixture: `git` });
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementationOnce(() => {
          // git repo check fails (not in git repo)
          throw new Error(
            "fatal: not a git repository (or any of the parent directories): .git"
          );
        })
        .mockImplementationOnce(() => {
          // hg repo check fails (not in hg repo)
          throw new Error("abort: no repository found (.hg not found)");
        })
        .mockReturnValue("success");

      const result = tryGitInit(root);
      expect(result).toBe(true);

      // Verify the exact sequence of commands (all with cwd: root)
      const calls = [
        GIT_REPO_COMMAND,
        HG_REPO_COMMAND,
        "git init",
        "git checkout -b main",
        "git add -A",
        'git commit -m "Initial commit from create-turbo"',
      ];
      expect(mockExecSync).toHaveBeenCalledTimes(calls.length);
      calls.forEach((call) => {
        expect(mockExecSync).toHaveBeenCalledWith(call, {
          stdio: "ignore",
          cwd: root,
        });
      });
      mockExecSync.mockRestore();
    });

    it("creates exactly one commit with all changes", async () => {
      const { root } = useFixture({ fixture: `git` });
      const commitCalls: Array<string> = [];
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementation((command) => {
          const cmd = command.toString();
          if (cmd === GIT_REPO_COMMAND) {
            throw new Error(
              "fatal: not a git repository (or any of the parent directories): .git"
            );
          }
          if (cmd === HG_REPO_COMMAND) {
            throw new Error("abort: no repository found (.hg not found)");
          }
          if (cmd.startsWith("git commit")) {
            commitCalls.push(cmd);
          }
          return "success";
        });

      tryGitInit(root);

      // Should have exactly one commit call
      expect(commitCalls).toHaveLength(1);
      expect(commitCalls[0]).toBe(
        'git commit -m "Initial commit from create-turbo"'
      );
      mockExecSync.mockRestore();
    });

    it("runs all git commands in the project root directory", async () => {
      const { root } = useFixture({ fixture: `git` });
      const cwdValues: Array<string | undefined> = [];
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementation((command, options) => {
          const opts = options as { cwd?: string };
          cwdValues.push(opts.cwd);
          const cmd = command.toString();
          if (cmd === GIT_REPO_COMMAND) {
            throw new Error("not in git repo");
          }
          if (cmd === HG_REPO_COMMAND) {
            throw new Error("not in hg repo");
          }
          return "success";
        });

      tryGitInit(root);

      // All commands should have cwd set to root
      expect(cwdValues.every((cwd) => cwd === root)).toBe(true);
      mockExecSync.mockRestore();
    });

    it("skips init if already in a git repo", async () => {
      const { root } = useFixture({
        fixture: `git`,
      });
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockReturnValueOnce("true") // git repo check succeeds
        .mockReturnValue("success");

      const result = tryGitInit(root);
      expect(result).toBe(false);

      // Should only call git repo check
      expect(mockExecSync).toHaveBeenCalledTimes(1);
      expect(mockExecSync).toHaveBeenCalledWith(GIT_REPO_COMMAND, {
        stdio: "ignore",
        cwd: root,
      });
      mockExecSync.mockRestore();
    });

    it("returns false on unexpected error during init", async () => {
      const { root } = useFixture({ fixture: `git` });
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementationOnce(() => {
          // not in git repo
          throw new Error(
            "fatal: not a git repository (or any of the parent directories): .git"
          );
        })
        .mockImplementationOnce(() => {
          // not in hg repo
          throw new Error("abort: no repository found (.hg not found)");
        })
        .mockImplementationOnce(() => {
          // git init fails
          throw new Error("fatal: 128");
        });

      const result = tryGitInit(root);
      expect(result).toBe(false);

      const calls: Array<string> = [
        GIT_REPO_COMMAND,
        HG_REPO_COMMAND,
        "git init",
      ];

      expect(mockExecSync).toHaveBeenCalledTimes(calls.length);
      calls.forEach((call) => {
        expect(mockExecSync).toHaveBeenCalledWith(call, {
          stdio: "ignore",
          cwd: root,
        });
      });
      mockExecSync.mockRestore();
    });

    it("cleans up .git directory on failure after init", async () => {
      const { root } = useFixture({ fixture: `git` });
      const mockRmSync = jest.spyOn(fs, "rmSync").mockImplementation(() => {});
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementationOnce(() => {
          // not in git repo
          throw new Error(
            "fatal: not a git repository (or any of the parent directories): .git"
          );
        })
        .mockImplementationOnce(() => {
          // not in hg repo
          throw new Error("abort: no repository found (.hg not found)");
        })
        .mockReturnValueOnce("success") // git init succeeds
        .mockImplementationOnce(() => {
          // git checkout -b main fails
          throw new Error("fatal: could not checkout branch");
        });

      const result = tryGitInit(root);
      expect(result).toBe(false);

      // Should clean up the .git directory
      expect(mockRmSync).toHaveBeenCalledWith(path.join(root, ".git"), {
        recursive: true,
        force: true,
      });
      mockExecSync.mockRestore();
      mockRmSync.mockRestore();
    });

    it("cleans up .git directory when user has no git config (commit fails)", async () => {
      const { root } = useFixture({ fixture: `git` });
      const mockRmSync = jest.spyOn(fs, "rmSync").mockImplementation(() => {});
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementationOnce(() => {
          // not in git repo
          throw new Error(
            "fatal: not a git repository (or any of the parent directories): .git"
          );
        })
        .mockImplementationOnce(() => {
          // not in hg repo
          throw new Error("abort: no repository found (.hg not found)");
        })
        .mockReturnValueOnce("success") // git init
        .mockReturnValueOnce("success") // git checkout -b main
        .mockReturnValueOnce("success") // git add -A
        .mockImplementationOnce(() => {
          // git commit fails due to missing user config
          throw new Error(
            "fatal: unable to auto-detect email address (got 'user@localhost')"
          );
        });

      const result = tryGitInit(root);
      expect(result).toBe(false);

      // Should clean up the .git directory since init succeeded but commit failed
      expect(mockRmSync).toHaveBeenCalledWith(path.join(root, ".git"), {
        recursive: true,
        force: true,
      });
      mockExecSync.mockRestore();
      mockRmSync.mockRestore();
    });

    it("skips init if already in a mercurial repo", async () => {
      const { root } = useFixture({
        fixture: `git`,
      });
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementationOnce(() => {
          // not in git repo
          throw new Error(
            "fatal: not a git repository (or any of the parent directories): .git"
          );
        })
        .mockReturnValueOnce("true") // hg repo check succeeds (is in hg repo)
        .mockReturnValue("success");

      const result = tryGitInit(root);
      expect(result).toBe(false);

      // Should call git repo check, then hg repo check, then stop
      expect(mockExecSync).toHaveBeenCalledTimes(2);
      expect(mockExecSync).toHaveBeenCalledWith(GIT_REPO_COMMAND, {
        stdio: "ignore",
        cwd: root,
      });
      expect(mockExecSync).toHaveBeenCalledWith(HG_REPO_COMMAND, {
        stdio: "ignore",
        cwd: root,
      });
      mockExecSync.mockRestore();
    });
  });

  describe("removeGitDirectory", () => {
    const { useFixture } = setupTestFixtures({
      directory: path.join(__dirname, "../"),
      options: { emptyFixture: true },
    });

    it("attempts to remove .git directory", async () => {
      const { root } = useFixture({ fixture: `remove-git` });
      const mockRmSync = jest.spyOn(fs, "rmSync").mockImplementation(() => {});

      const result = removeGitDirectory(root);
      expect(result).toBe(true);

      expect(mockRmSync).toHaveBeenCalledWith(path.join(root, ".git"), {
        recursive: true,
        force: true,
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
        force: true,
      });
      mockRmSync.mockRestore();
    });
  });
});
