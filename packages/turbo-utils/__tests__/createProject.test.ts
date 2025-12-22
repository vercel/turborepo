import { describe, it, expect } from "@jest/globals";
import { isValidGitHubRepoUrl } from "../src/createProject";

describe("createProject", () => {
  describe("isValidGitHubRepoUrl (SSRF protection)", () => {
    it("allows valid github.com URLs", () => {
      expect(
        isValidGitHubRepoUrl(new URL("https://github.com/vercel/turbo"))
      ).toBe(true);
      expect(
        isValidGitHubRepoUrl(new URL("https://github.com/user/repo"))
      ).toBe(true);
      expect(
        isValidGitHubRepoUrl(new URL("https://github.com/org/repo/tree/main"))
      ).toBe(true);
    });

    it("allows github.com with different protocols", () => {
      expect(isValidGitHubRepoUrl(new URL("http://github.com/user/repo"))).toBe(
        true
      );
    });

    it("blocks SSRF bypass attempts with subdomain spoofing", () => {
      // github.com.evil.com - attacker-controlled domain
      expect(
        isValidGitHubRepoUrl(new URL("https://github.com.evil.com/repo"))
      ).toBe(false);
      // github.com.attacker.io
      expect(
        isValidGitHubRepoUrl(new URL("https://github.com.attacker.io/repo"))
      ).toBe(false);
    });

    it("blocks non-GitHub hosts", () => {
      expect(
        isValidGitHubRepoUrl(new URL("https://gitlab.com/user/repo"))
      ).toBe(false);
      expect(
        isValidGitHubRepoUrl(new URL("https://bitbucket.org/user/repo"))
      ).toBe(false);
      expect(
        isValidGitHubRepoUrl(new URL("https://example.com/user/repo"))
      ).toBe(false);
    });

    it("blocks GitHub Enterprise or other GitHub subdomains", () => {
      // These might be legitimate but are not github.com
      expect(
        isValidGitHubRepoUrl(new URL("https://api.github.com/repos"))
      ).toBe(false);
      expect(
        isValidGitHubRepoUrl(new URL("https://raw.githubusercontent.com/file"))
      ).toBe(false);
      expect(
        isValidGitHubRepoUrl(new URL("https://gist.github.com/user/id"))
      ).toBe(false);
    });

    it("blocks URLs with credentials that might bypass checks", () => {
      // URL with userinfo: https://github.com@evil.com/repo
      // The hostname here is evil.com, not github.com
      expect(
        isValidGitHubRepoUrl(new URL("https://github.com@evil.com/repo"))
      ).toBe(false);
    });

    it("blocks localhost and internal network URLs", () => {
      expect(isValidGitHubRepoUrl(new URL("http://localhost/repo"))).toBe(
        false
      );
      expect(isValidGitHubRepoUrl(new URL("http://127.0.0.1/repo"))).toBe(
        false
      );
      expect(isValidGitHubRepoUrl(new URL("http://192.168.1.1/repo"))).toBe(
        false
      );
      expect(isValidGitHubRepoUrl(new URL("http://10.0.0.1/repo"))).toBe(false);
    });

    it("blocks file:// protocol URLs", () => {
      expect(() =>
        isValidGitHubRepoUrl(new URL("file:///etc/passwd"))
      ).not.toThrow();
      expect(isValidGitHubRepoUrl(new URL("file:///etc/passwd"))).toBe(false);
    });
  });
});
