import assert from "node:assert/strict";
import { createHmac } from "node:crypto";
import test from "node:test";

import {
  MAIN_REF,
  parseGitHubPush,
  TURBOREPO_REPOSITORY,
  verifyGitHubSignature
} from "../agent/lib/github-push.ts";

const SECRET = "factory-image-secret";
const COMMIT = "0123456789abcdef0123456789abcdef01234567";

function sign(body, secret = SECRET) {
  return `sha256=${createHmac("sha256", secret).update(body).digest("hex")}`;
}

function pushBody(overrides = {}) {
  return JSON.stringify({
    after: COMMIT,
    pusher: { name: "turbobot" },
    ref: MAIN_REF,
    repository: { full_name: TURBOREPO_REPOSITORY },
    ...overrides
  });
}

test("only a matching signature is accepted", () => {
  const body = pushBody();
  assert.equal(verifyGitHubSignature(body, sign(body), SECRET), true);
  assert.equal(
    verifyGitHubSignature(body, sign(body, "other-secret"), SECRET),
    false
  );
  assert.equal(verifyGitHubSignature(`${body} `, sign(body), SECRET), false);
  assert.equal(verifyGitHubSignature(body, null, SECRET), false);
  assert.equal(verifyGitHubSignature(body, "", SECRET), false);
  assert.equal(verifyGitHubSignature(body, "sha256=short", SECRET), false);
  assert.equal(
    verifyGitHubSignature(body, sign(body).replace("sha256=", ""), SECRET),
    false
  );
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

test("pings are acknowledged without building", () => {
  assert.equal(parseGitHubPush("ping", "{}").kind, "ping");
});

test("everything else is ignored with a reason", () => {
  const cases = [
    ["issue_comment", pushBody(), /Unsupported event/],
    [undefined, pushBody(), /Unsupported event/],
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
