import assert from "node:assert/strict";
import test from "node:test";

import {
  assertNoReleaseAgeExclusion,
  findReleaseAgeExclusion,
  isReleaseAgeConfigFile
} from "../agent/lib/release-age.ts";

test("recognizes package-manager configuration files", () => {
  for (const file of [
    "examples/basic/pnpm-workspace.yaml",
    "examples/basic/.npmrc",
    "examples/with-berry/.yarnrc.yml",
    "examples/with-nextjs-elysia/bunfig.toml",
    "examples/basic/apps/web/package.json"
  ]) {
    assert.equal(isReleaseAgeConfigFile(file), true, file);
  }

  for (const file of [
    "examples/basic/README.md",
    "examples/basic/turbo.json",
    "examples/basic/apps/web/next.config.js"
  ]) {
    assert.equal(isReleaseAgeConfigFile(file), false, file);
  }
});

test("flags release-age exclusion lists in every package-manager dialect", () => {
  assert.deepEqual(
    findReleaseAgeExclusion(
      "examples/basic/pnpm-workspace.yaml",
      'packages:\n  - "apps/*"\n\nminimumReleaseAgeExclude:\n  - next\n'
    ),
    { line: 4, text: "minimumReleaseAgeExclude:" }
  );
  assert.deepEqual(
    findReleaseAgeExclusion(
      "examples/with-nextjs-elysia/bunfig.toml",
      '[install]\nminimumReleaseAgeExcludes = ["next"]\n'
    ),
    { line: 2, text: 'minimumReleaseAgeExcludes = ["next"]' }
  );
  assert.deepEqual(
    findReleaseAgeExclusion(
      "examples/with-npm/.npmrc",
      "minimum-release-age-exclude=next\n"
    ),
    { line: 1, text: "minimum-release-age-exclude=next" }
  );
  assert.deepEqual(
    findReleaseAgeExclusion(
      "examples/basic/package.json",
      '{\n  "pnpm": {\n    "minimumReleaseAgeExclude": ["next"]\n  }\n}\n'
    ),
    { line: 3, text: '"minimumReleaseAgeExclude": ["next"]' }
  );
});

test("allows unrelated configuration and non-configuration files", () => {
  assert.equal(
    findReleaseAgeExclusion(
      "examples/basic/pnpm-workspace.yaml",
      'packages:\n  - "apps/*"\n  - "packages/*"\n'
    ),
    null
  );
  assert.equal(
    findReleaseAgeExclusion(
      "examples/basic/README.md",
      "This example does not use minimumReleaseAgeExclude.\n"
    ),
    null
  );
});

test("leaves plain minimumReleaseAge settings alone", () => {
  assert.equal(
    findReleaseAgeExclusion(
      "examples/basic/pnpm-workspace.yaml",
      "minimumReleaseAge: 2880\n"
    ),
    null
  );
});

test("rejects writes that introduce a release-age exclusion", () => {
  assert.throws(
    () =>
      assertNoReleaseAgeExclusion(
        "examples/basic/pnpm-workspace.yaml",
        'packages:\n  - "apps/*"\n\nminimumReleaseAgeExclude:\n  - next\n'
      ),
    /examples\/basic\/pnpm-workspace\.yaml:4 adds a release-age exclusion/
  );
  assert.doesNotThrow(() =>
    assertNoReleaseAgeExclusion(
      "examples/basic/pnpm-workspace.yaml",
      'packages:\n  - "apps/*"\n'
    )
  );
});
