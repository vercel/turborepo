import {
  describe,
  it,
  expect,
  jest,
  beforeEach,
  afterEach,
} from "@jest/globals";
import { Readable, PassThrough } from "node:stream";
import { createGzip } from "node:zlib";
import {
  mkdirSync,
  rmSync,
  existsSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import * as tar from "tar";
import {
  isUrlOk,
  getRepoInfo,
  hasRepo,
  isPathSafe,
  isLinkEntry,
  streamingExtract,
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

    it("uses proxy agent when https_proxy is set", async () => {
      const originalEnv = process.env.https_proxy;
      process.env.https_proxy = "http://proxy.example.com:8080";

      try {
        global.fetch = jest.fn(() =>
          Promise.resolve({ ok: true } as Response)
        ) as typeof fetch;

        const url = "https://github.com/vercel/turborepo/";
        await isUrlOk(url);

        // Verify that fetch was called with a dispatcher option
        expect(global.fetch).toHaveBeenCalledWith(
          url,
          expect.objectContaining({
            method: "HEAD",
            dispatcher: expect.anything(),
          })
        );
      } finally {
        if (originalEnv === undefined) {
          delete process.env.https_proxy;
        } else {
          process.env.https_proxy = originalEnv;
        }
      }
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
  });

  describe("isPathSafe with pre-resolved root (performance optimization)", () => {
    it("works with pre-resolved root parameter", () => {
      const resolvedRoot = "/tmp/extract";
      expect(isPathSafe("/tmp/extract", "file.txt", resolvedRoot)).toBe(true);
      expect(isPathSafe("/tmp/extract", "../etc/passwd", resolvedRoot)).toBe(
        false
      );
    });
  });

  describe("streamingExtract", () => {
    let testDir: string;
    let sourceDir: string;

    beforeEach(() => {
      const baseDir = join(
        tmpdir(),
        `turbo-test-${Date.now()}-${Math.random().toString(36).slice(2)}`
      );
      testDir = join(baseDir, "extract");
      sourceDir = join(baseDir, "source");
      mkdirSync(testDir, { recursive: true });
      mkdirSync(sourceDir, { recursive: true });
    });

    afterEach(() => {
      // Clean up both directories
      const baseDir = join(testDir, "..");
      rmSync(baseDir, { recursive: true, force: true });
    });

    /**
     * Helper to create a mock tarball response body from a directory structure
     */
    async function createMockTarballBody(
      files: Array<{
        path: string;
        content?: string;
        type?: "file" | "directory";
      }>
    ): Promise<ReadableStream<Uint8Array>> {
      // Create the source structure
      const tarSourceDir = join(sourceDir, "tarroot");
      mkdirSync(tarSourceDir, { recursive: true });

      for (const file of files) {
        const fullPath = join(tarSourceDir, file.path);
        if (file.type === "directory") {
          mkdirSync(fullPath, { recursive: true });
        } else {
          mkdirSync(join(fullPath, ".."), { recursive: true });
          writeFileSync(fullPath, file.content ?? "");
        }
      }

      // Create tarball stream
      const passThrough = new PassThrough();

      tar
        .create(
          {
            gzip: true,
            cwd: sourceDir,
          },
          ["tarroot"]
        )
        .pipe(passThrough);

      const nodeReadable = Readable.from(passThrough);
      return Readable.toWeb(nodeReadable) as ReadableStream<Uint8Array>;
    }

    it("extracts files from a tarball successfully", async () => {
      const mockBody = await createMockTarballBody([
        { path: "file.txt", content: "Hello World" },
        { path: "subdir", type: "directory" },
        { path: "subdir/nested.txt", content: "Nested content" },
      ]);

      global.fetch = jest.fn(() =>
        Promise.resolve({
          ok: true,
          body: mockBody,
        } as Response)
      ) as typeof fetch;

      await streamingExtract({
        url: "https://example.com/tarball.tar.gz",
        root: testDir,
        strip: 1,
        filter: () => true,
      });

      expect(existsSync(join(testDir, "file.txt"))).toBe(true);
      expect(readFileSync(join(testDir, "file.txt"), "utf-8")).toBe(
        "Hello World"
      );
      expect(existsSync(join(testDir, "subdir", "nested.txt"))).toBe(true);
      expect(readFileSync(join(testDir, "subdir", "nested.txt"), "utf-8")).toBe(
        "Nested content"
      );
    });

    it("throws error on failed download (non-ok response)", async () => {
      global.fetch = jest.fn(() =>
        Promise.resolve({
          ok: false,
          status: 404,
          body: null,
        } as Response)
      ) as typeof fetch;

      await expect(
        streamingExtract({
          url: "https://example.com/notfound.tar.gz",
          root: testDir,
          strip: 1,
          filter: () => true,
        })
      ).rejects.toThrow("Failed to download: 404");
    });

    it("throws error when response body is null", async () => {
      global.fetch = jest.fn(() =>
        Promise.resolve({
          ok: true,
          body: null,
        } as Response)
      ) as typeof fetch;

      await expect(
        streamingExtract({
          url: "https://example.com/nobody.tar.gz",
          root: testDir,
          strip: 1,
          filter: () => true,
        })
      ).rejects.toThrow("Failed to download");
    });

    it("respects filter function", async () => {
      const mockBody = await createMockTarballBody([
        { path: "include.txt", content: "Included" },
        { path: "exclude.txt", content: "Excluded" },
      ]);

      global.fetch = jest.fn(() =>
        Promise.resolve({
          ok: true,
          body: mockBody,
        } as Response)
      ) as typeof fetch;

      await streamingExtract({
        url: "https://example.com/tarball.tar.gz",
        root: testDir,
        strip: 1,
        filter: (p: string) => p.includes("include"),
      });

      expect(existsSync(join(testDir, "include.txt"))).toBe(true);
      expect(existsSync(join(testDir, "exclude.txt"))).toBe(false);
    });

    it("handles network errors gracefully", async () => {
      global.fetch = jest.fn(() =>
        Promise.reject(new Error("Network error"))
      ) as typeof fetch;

      await expect(
        streamingExtract({
          url: "https://example.com/tarball.tar.gz",
          root: testDir,
          strip: 1,
          filter: () => true,
        })
      ).rejects.toThrow("Network error");
    });

    it("strips correct number of path components", async () => {
      const mockBody = await createMockTarballBody([
        { path: "examples", type: "directory" },
        { path: "examples/basic", type: "directory" },
        { path: "examples/basic/package.json", content: "{}" },
      ]);

      global.fetch = jest.fn(() =>
        Promise.resolve({
          ok: true,
          body: mockBody,
        } as Response)
      ) as typeof fetch;

      await streamingExtract({
        url: "https://example.com/tarball.tar.gz",
        root: testDir,
        strip: 3, // Strip "tarroot/examples/basic"
        filter: (p: string) => p.includes("examples/basic/"),
      });

      expect(existsSync(join(testDir, "package.json"))).toBe(true);
    });

    it("extracts nested directory structures correctly", async () => {
      const mockBody = await createMockTarballBody([
        { path: "a", type: "directory" },
        { path: "a/b", type: "directory" },
        { path: "a/b/c", type: "directory" },
        { path: "a/b/c/deep.txt", content: "Deep file" },
      ]);

      global.fetch = jest.fn(() =>
        Promise.resolve({
          ok: true,
          body: mockBody,
        } as Response)
      ) as typeof fetch;

      await streamingExtract({
        url: "https://example.com/tarball.tar.gz",
        root: testDir,
        strip: 1,
        filter: () => true,
      });

      expect(existsSync(join(testDir, "a", "b", "c", "deep.txt"))).toBe(true);
      expect(
        readFileSync(join(testDir, "a", "b", "c", "deep.txt"), "utf-8")
      ).toBe("Deep file");
    });

    it("uses proxy agent when HTTPS_PROXY is set", async () => {
      const originalEnv = process.env.HTTPS_PROXY;
      process.env.HTTPS_PROXY = "http://proxy.example.com:8080";

      try {
        const mockBody = await createMockTarballBody([
          { path: "file.txt", content: "Hello" },
        ]);

        global.fetch = jest.fn(() =>
          Promise.resolve({
            ok: true,
            body: mockBody,
          } as Response)
        ) as typeof fetch;

        await streamingExtract({
          url: "https://example.com/tarball.tar.gz",
          root: testDir,
          strip: 1,
          filter: () => true,
        });

        // Verify that fetch was called with a dispatcher option
        expect(global.fetch).toHaveBeenCalledWith(
          "https://example.com/tarball.tar.gz",
          expect.objectContaining({
            dispatcher: expect.anything(),
          })
        );
      } finally {
        if (originalEnv === undefined) {
          delete process.env.HTTPS_PROXY;
        } else {
          process.env.HTTPS_PROXY = originalEnv;
        }
      }
    });

    it("does not use proxy agent when no proxy env vars are set", async () => {
      const originalHttpsProxy = process.env.HTTPS_PROXY;
      const originalHttpProxy = process.env.HTTP_PROXY;
      const originalHttpsProxyLower = process.env.https_proxy;
      const originalHttpProxyLower = process.env.http_proxy;

      delete process.env.HTTPS_PROXY;
      delete process.env.HTTP_PROXY;
      delete process.env.https_proxy;
      delete process.env.http_proxy;

      try {
        const mockBody = await createMockTarballBody([
          { path: "file.txt", content: "Hello" },
        ]);

        global.fetch = jest.fn(() =>
          Promise.resolve({
            ok: true,
            body: mockBody,
          } as Response)
        ) as typeof fetch;

        await streamingExtract({
          url: "https://example.com/tarball.tar.gz",
          root: testDir,
          strip: 1,
          filter: () => true,
        });

        // Verify that fetch was called with undefined dispatcher
        expect(global.fetch).toHaveBeenCalledWith(
          "https://example.com/tarball.tar.gz",
          expect.objectContaining({
            dispatcher: undefined,
          })
        );
      } finally {
        if (originalHttpsProxy !== undefined)
          process.env.HTTPS_PROXY = originalHttpsProxy;
        if (originalHttpProxy !== undefined)
          process.env.HTTP_PROXY = originalHttpProxy;
        if (originalHttpsProxyLower !== undefined)
          process.env.https_proxy = originalHttpsProxyLower;
        if (originalHttpProxyLower !== undefined)
          process.env.http_proxy = originalHttpProxyLower;
      }
    });
  });
});
