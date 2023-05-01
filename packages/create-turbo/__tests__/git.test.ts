import path from "path";
import {
  DEFAULT_IGNORE,
  GIT_REPO_COMMAND,
  HG_REPO_COMMAND,
  isInGitRepository,
  isInMercurialRepository,
  tryGitInit,
} from "../src/utils/git";
import childProcess from "child_process";
import { setupTestFixtures } from "@turbo/test-utils";

describe("git", () => {
  // just to make sure this doesn't get lost
  it("default .gitignore includes .turbo", async () => {
    expect(DEFAULT_IGNORE).toContain(".turbo");
  });

  describe("isInGitRepository", () => {
    it("returns true when in a repo", async () => {
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockReturnValue("true");

      const result = isInGitRepository();
      expect(result).toBe(true);

      expect(mockExecSync).toHaveBeenCalledWith(GIT_REPO_COMMAND, {
        stdio: "ignore",
      });
      mockExecSync.mockRestore();
    });

    it("returns false when not in a repo", async () => {
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementation(() => {
          throw new Error(
            "fatal: not a git repository (or any of the parent directories): .git"
          );
        });

      const result = isInGitRepository();
      expect(result).toBe(false);

      expect(mockExecSync).toHaveBeenCalledWith(GIT_REPO_COMMAND, {
        stdio: "ignore",
      });
      mockExecSync.mockRestore();
    });

    it("returns false on error", async () => {
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementation(() => {
          throw new Error("unknown error");
        });

      const result = isInGitRepository();
      expect(result).toBe(false);

      expect(mockExecSync).toHaveBeenCalledWith(GIT_REPO_COMMAND, {
        stdio: "ignore",
      });
      mockExecSync.mockRestore();
    });
  });

  describe("isInMercurialRepository", () => {
    it("returns true when in a repo", async () => {
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockReturnValue("true");

      const result = isInMercurialRepository();
      expect(result).toBe(true);

      expect(mockExecSync).toHaveBeenCalledWith(HG_REPO_COMMAND, {
        stdio: "ignore",
      });
      mockExecSync.mockRestore();
    });

    it("returns false when not in a repo", async () => {
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementation(() => {
          throw new Error("abort: no repository found (.hg not found)");
        });

      const result = isInMercurialRepository();
      expect(result).toBe(false);

      expect(mockExecSync).toHaveBeenCalledWith(HG_REPO_COMMAND, {
        stdio: "ignore",
      });
      mockExecSync.mockRestore();
    });

    it("returns false on error", async () => {
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementation(() => {
          throw new Error("unknown error");
        });

      const result = isInMercurialRepository();
      expect(result).toBe(false);

      expect(mockExecSync).toHaveBeenCalledWith(HG_REPO_COMMAND, {
        stdio: "ignore",
      });
      mockExecSync.mockRestore();
    });
  });

  describe("tryGitInit", () => {
    const { useFixture } = setupTestFixtures({
      directory: path.join(__dirname, "../"),
    });

    it("inits a repo succesfully", async () => {
      const { root } = useFixture({ fixture: `git` });
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockReturnValueOnce("git version 2.38.1")
        .mockImplementationOnce(() => {
          throw new Error(
            "fatal: not a git repository (or any of the parent directories): .git"
          );
        })
        .mockImplementationOnce(() => {
          throw new Error("abort: no repository found (.hg not found)");
        })
        .mockReturnValue("success");

      const result = tryGitInit(root, "test commit");
      expect(result).toBe(true);

      const calls = [
        "git --version",
        "git init",
        "git checkout -b main",
        "git add -A",
        'git commit -m "test commit"',
      ];
      expect(mockExecSync).toHaveBeenCalledTimes(calls.length + 2);
      calls.forEach((call) => {
        expect(mockExecSync).toHaveBeenCalledWith(call, {
          stdio: "ignore",
        });
      });
      mockExecSync.mockRestore();
    });

    it("skips init if already in a repo", async () => {
      const { root } = useFixture({ fixture: `git` });
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockReturnValueOnce("git version 2.38.1")
        .mockReturnValueOnce("true")
        .mockReturnValue("success");

      const result = tryGitInit(root, "test commit");
      expect(result).toBe(false);

      const calls = ["git --version"];

      // 1 call for git --version, 1 call for isInGitRepository
      expect(mockExecSync).toHaveBeenCalledTimes(calls.length + 1);
      calls.forEach((call) => {
        expect(mockExecSync).toHaveBeenCalledWith(call, {
          stdio: "ignore",
        });
      });
      mockExecSync.mockRestore();
    });

    it("returns false on unexpected error", async () => {
      const { root } = useFixture({ fixture: `git` });
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockImplementationOnce(() => {
          throw new Error("fatal: unknown command git");
        });

      const result = tryGitInit(root, "test commit");
      expect(result).toBe(false);

      const calls = ["git --version"];

      expect(mockExecSync).toHaveBeenCalledTimes(calls.length);
      calls.forEach((call) => {
        expect(mockExecSync).toHaveBeenCalledWith(call, {
          stdio: "ignore",
        });
      });
      mockExecSync.mockRestore();
    });

    it("cleans up from partial init on failure", async () => {
      const { root } = useFixture({ fixture: `git` });
      const mockExecSync = jest
        .spyOn(childProcess, "execSync")
        .mockReturnValueOnce("git version 2.38.1")
        .mockImplementationOnce(() => {
          throw new Error(
            "fatal: not a git repository (or any of the parent directories): .git"
          );
        })
        .mockImplementationOnce(() => {
          throw new Error("abort: no repository found (.hg not found)");
        })
        .mockReturnValueOnce("success")
        .mockReturnValueOnce("success")
        .mockImplementationOnce(() => {
          throw new Error("fatal: could not add files");
        });

      const result = tryGitInit(root, "test commit");
      expect(result).toBe(false);

      const calls = [
        "git --version",
        "git init",
        "git checkout -b main",
        "git add -A",
      ];

      expect(mockExecSync).toHaveBeenCalledTimes(calls.length + 2);
      calls.forEach((call) => {
        expect(mockExecSync).toHaveBeenCalledWith(call, {
          stdio: "ignore",
        });
      });
      mockExecSync.mockRestore();
    });
  });
});
