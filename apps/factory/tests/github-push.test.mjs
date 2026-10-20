import assert from "node:assert/strict";
import test from "node:test";

import {
  isFactoryImageConnector,
  MAIN_REF,
  parseGitHubPush,
  TURBOREPO_REPOSITORY
} from "../agent/lib/github-push.ts";

const COMMIT = "0123456789abcdef0123456789abcdef01234567";

function pushBody(overrides = {}) {
  return JSON.stringify({
    after: COMMIT,
    pusher: { name: "turbobot" },
    ref: MAIN_REF,
    repository: { full_name: TURBOREPO_REPOSITORY },
    ...overrides
  });
}

test("only the configured Connect connector is accepted", () => {
  assert.equal(
    isFactoryImageConnector(
      { attributes: { connector_id: "scl_factory" } },
      "scl_factory"
    ),
    true
  );
  assert.equal(
    isFactoryImageConnector(
      { attributes: { connector_id: "scl_other" } },
      "scl_factory"
    ),
    false
  );
  assert.equal(isFactoryImageConnector({ attributes: {} }, undefined), false);
  assert.equal(isFactoryImageConnector(null, "scl_factory"), false);
});

test("a merge to main starts a build", () => {
  const outcome = parseGitHubPush("push", pushBody());
  assert.equal(outcome.kind, "push");
  assert.deepEqual(outcome.push, {
    commit: COMMIT,
    pusher: "turbobot",
    ref: MAIN_REF
  });
});

test("a Connect-forwarded push does not require the GitHub event header", () => {
  assert.equal(parseGitHubPush(undefined, pushBody()).kind, "push");
});

test("pings are acknowledged without building", () => {
  assert.equal(parseGitHubPush("ping", "{}").kind, "ping");
});

test("everything else is ignored with a reason", () => {
  const cases = [
    ["issue_comment", pushBody(), /Unsupported event/],
    [undefined, JSON.stringify({ action: "opened" }), /Unsupported event/],
    ["push", "not json", /not valid JSON/],
    ["push", "[]", /not an object/],
    [
      "push",
      pushBody({ ref: "refs/heads/release" }),
      /Not a refs\/heads\/main/
    ],
    ["push", pushBody({ ref: "refs/tags/v1.0.0" }), /Not a refs\/heads\/main/],
    ["push", pushBody({ deleted: true }), /branch was deleted/],
    ["push", pushBody({ after: "0".repeat(40) }), /no head commit/],
    ["push", pushBody({ after: "not-a-sha" }), /no head commit/],
    ["push", pushBody({ after: undefined }), /no head commit/],
    [
      "push",
      pushBody({ repository: { full_name: "attacker/turborepo" } }),
      /Unexpected repository/
    ],
    ["push", pushBody({ repository: undefined }), /Unexpected repository/]
  ];
  for (const [event, body, reason] of cases) {
    const outcome = parseGitHubPush(event, body);
    assert.equal(outcome.kind, "ignored", `${event}: ${body.slice(0, 40)}`);
    assert.match(outcome.reason, reason);
  }
});
