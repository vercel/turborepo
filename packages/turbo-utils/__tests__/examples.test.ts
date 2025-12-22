import {
  describe,
  it,
  expect,
  jest,
  beforeEach,
  afterEach,
} from "@jest/globals";
import {
  isUrlOk,
  getRepoInfo,
  hasRepo,
  isPathSafe,
  isLinkEntry,
} from "../src/examples";

describe("examples", () => {
  const originalFetch = global.fetch;

  afterEach(() => {
    global.fetch = originalFetch;
  });

  describe("isUrlOk", () => {
    it("returns true if url returns 200", async () => {
      global.fetch = jest.fn(() =>
        Promise.resolve({ ok: true } as Response)
      ) as typeof fetch;

      const url = "https://github.com/vercel/turborepo/";
      const result = await isUrlOk(url);
      expect(result).toBe(true);

      expect(global.fetch).toHaveBeenCalledWith(
        url,
        expect.objectContaining({ method: "HEAD" })
      );
    });

    it("returns false if url returns status != 200", async () => {
      global.fetch = jest.fn(() =>
        Promise.resolve({ ok: false } as Response)
      ) as typeof fetch;

      const url = "https://not-github.com/vercel/turborepo/";
      const result = await isUrlOk(url);
      expect(result).toBe(false);

      expect(global.fetch).toHaveBeenCalledWith(
        url,
        expect.objectContaining({ method: "HEAD" })
      );
    });
  });

  describe("getRepoInfo", () => {
    it.each([
      {
        repoUrl: "https://github.com/vercel/turborepo/",
        examplePath: undefined,
        defaultBranch: "main",
        expectBranchLookup: true,
        expected: {
          username: "vercel",
          name: "turborepo",
          branch: "main",
          filePath: "",
        },
      },
      {
        repoUrl:
          "https://github.com/vercel/turborepo/tree/canary/examples/kitchen-sink",
        examplePath: undefined,
        defaultBranch: "canary",
        expectBranchLookup: false,
        expected: {
          username: "vercel",
          name: "turborepo",
          branch: "canary",
          filePath: "examples/kitchen-sink",
        },
      },
      {
        repoUrl: "https://github.com/vercel/turborepo/tree/tek/test-branch/",
        examplePath: "examples/basic",
        defaultBranch: "canary",
        expectBranchLookup: false,
        expected: {
          username: "vercel",
          name: "turborepo",
          branch: "tek/test-branch",
          filePath: "examples/basic",
        },
      },
    ])(
      "retrieves repo info for $repoUrl and $examplePath",
      async ({
        repoUrl,
        examplePath,
        defaultBranch,
        expectBranchLookup,
        expected,
      }) => {
        global.fetch = jest.fn(() =>
          Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ default_branch: defaultBranch }),
          } as Response)
        ) as typeof fetch;

        const url = new URL(repoUrl);
        const result = await getRepoInfo(url, examplePath);
        expect(result).toMatchObject(expected);

        if (result && expectBranchLookup) {
          expect(global.fetch).toHaveBeenCalledWith(
            `https://api.github.com/repos/${result.username}/${result.name}`,
            expect.any(Object)
          );
        }
      }
    );
  });

  describe("hasRepo", () => {
    it.each([
      {
        repoInfo: {
          username: "vercel",
          name: "turbo",
          branch: "main",
          filePath: "",
        },
        expected: true,
        expectedUrl:
          "https://api.github.com/repos/vercel/turbo/contents/package.json?ref=main",
      },
    ])(
      "checks repo at $expectedUrl",
      async ({ expected, repoInfo, expectedUrl }) => {
        global.fetch = jest.fn(() =>
          Promise.resolve({ ok: true } as Response)
        ) as typeof fetch;

        const result = await hasRepo(repoInfo);
        expect(result).toBe(expected);

        expect(global.fetch).toHaveBeenCalledWith(
          expectedUrl,
          expect.objectContaining({ method: "HEAD" })
        );
      }
    );
  });

  describe("isPathSafe (Zip Slip protection)", () => {
    it("allows paths within the root directory", () => {
      expect(isPathSafe("/tmp/extract", "file.txt")).toBe(true);
      expect(isPathSafe("/tmp/extract", "subdir/file.txt")).toBe(true);
      expect(isPathSafe("/tmp/extract", "a/b/c/file.txt")).toBe(true);
    });

    it("blocks path traversal with ../", () => {
      expect(isPathSafe("/tmp/extract", "../etc/passwd")).toBe(false);
      expect(isPathSafe("/tmp/extract", "../../etc/passwd")).toBe(false);
      expect(isPathSafe("/tmp/extract", "../../../etc/passwd")).toBe(false);
    });

    it("blocks path traversal hidden in nested paths", () => {
      expect(isPathSafe("/tmp/extract", "foo/../../../etc/passwd")).toBe(false);
      expect(isPathSafe("/tmp/extract", "foo/bar/../../../etc/passwd")).toBe(
        false
      );
    });

    it("allows paths that contain .. but stay within root", () => {
      // foo/../bar resolves to just "bar" which is still within root
      expect(isPathSafe("/tmp/extract", "foo/../bar")).toBe(true);
      expect(isPathSafe("/tmp/extract", "a/b/../c")).toBe(true);
    });

    it("blocks absolute paths that escape root", () => {
      // An absolute path would resolve to itself, escaping the root
      expect(isPathSafe("/tmp/extract", "/etc/passwd")).toBe(false);
    });
  });

  describe("isLinkEntry (symlink attack protection)", () => {
    it("identifies symbolic links", () => {
      expect(isLinkEntry("SymbolicLink")).toBe(true);
    });

    it("identifies hard links", () => {
      expect(isLinkEntry("Link")).toBe(true);
    });

    it("allows regular files", () => {
      expect(isLinkEntry("File")).toBe(false);
    });

    it("allows directories", () => {
      expect(isLinkEntry("Directory")).toBe(false);
    });

    it("allows other entry types", () => {
      expect(isLinkEntry("BlockDevice")).toBe(false);
      expect(isLinkEntry("CharacterDevice")).toBe(false);
      expect(isLinkEntry("FIFO")).toBe(false);
    });
  });
});
