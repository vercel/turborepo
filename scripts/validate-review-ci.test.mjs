import assert from "node:assert/strict";
import test from "node:test";

import { validateReviewCI } from "./validate-review-ci.mjs";

const ROOT = "/repos/vercel/turborepo";
const WORKFLOW = ".github/workflows/turborepo-test.yml";
const AGGREGATE = "Code checks (PR #42, main)";
const TIMEOUT = 30 * 60 * 1000;
const clone = (value) => structuredClone(value);
const repo = () => ({
  id: 1,
  full_name: "vercel/turborepo",
  default_branch: "main"
});
const fork = () => ({ id: 2, full_name: "contributor/turborepo" });
const association = () => ({
  number: 42,
  head: { sha: "head-sha", ref: "feature", repo: fork() },
  base: { sha: "base-sha", ref: "main", repo: repo() }
});
const aggregate = () => ({
  name: AGGREGATE,
  status: "completed",
  conclusion: "success"
});

function codeRun(overrides = {}) {
  return {
    id: 100,
    run_number: 10,
    run_attempt: 1,
    workflow_id: 9,
    path: WORKFLOW,
    event: "pull_request",
    head_sha: "head-sha",
    head_branch: "feature",
    head_repository: fork(),
    repository: repo(),
    pull_requests: [association()],
    status: "completed",
    conclusion: "success",
    ...overrides
  };
}

// Every response is cloned: mutations represent later GitHub responses, not
// changes to an already-returned snapshot. Hooks run before endpoint dispatch.
function fixture(t) {
  const f = {
    clock: 1_000_000,
    calls: [],
    delays: [],
    counts: new Map(),
    pull: {
      number: 42,
      state: "open",
      draft: false,
      head: { sha: "head-sha", ref: "feature", repo: fork() },
      base: { sha: "base-sha", ref: "main", repo: repo() }
    },
    review: codeRun({ id: 200, run_number: 11, event: "pull_request_review" }),
    runs: [codeRun()],
    details: new Map(),
    jobs: new Map([["100/1", [aggregate()]]]),
    workflowPages: undefined,
    jobPages: undefined,
    onRequest: undefined,
    onWait: undefined
  };
  t.mock.method(Date, "now", () => f.clock);

  f.request = async (path) => {
    const url = new URL(path, "https://api.github.com");
    assert.equal(url.origin, "https://api.github.com");
    assert.equal(
      path,
      `${url.pathname}${url.search}`,
      "request must use a relative API path"
    );
    let kind;
    let id;
    let attempt;
    let page;
    if (url.pathname === `${ROOT}/pulls/42`) {
      kind = "pull";
      assert.equal(url.search, "");
    } else if (url.pathname === `${ROOT}/actions/workflows/9/runs`) {
      kind = "workflow";
      assert.deepEqual([...url.searchParams.keys()].sort(), [
        "event",
        "head_sha",
        "page",
        "per_page"
      ]);
      assert.equal(url.searchParams.get("event"), "pull_request");
      assert.equal(url.searchParams.get("head_sha"), "head-sha");
      assert.equal(url.searchParams.get("per_page"), "100");
      page = Number(url.searchParams.get("page"));
      assert.ok(Number.isSafeInteger(page) && page > 0);
    } else {
      const run = url.pathname.match(
        /^\/repos\/vercel\/turborepo\/actions\/runs\/(\d+)$/
      );
      const jobs = url.pathname.match(
        /^\/repos\/vercel\/turborepo\/actions\/runs\/(\d+)\/attempts\/(\d+)\/jobs$/
      );
      if (run) {
        id = Number(run[1]);
        kind = id === 200 ? "review" : "detail";
        assert.equal(url.search, "");
      } else if (jobs) {
        kind = "jobs";
        id = Number(jobs[1]);
        attempt = Number(jobs[2]);
        assert.deepEqual([...url.searchParams.keys()].sort(), [
          "page",
          "per_page"
        ]);
        assert.equal(url.searchParams.get("per_page"), "100");
        page = Number(url.searchParams.get("page"));
        assert.ok(Number.isSafeInteger(page) && page > 0);
      } else {
        assert.fail(`Unexpected API endpoint: ${path}`);
      }
    }
    const count = (f.counts.get(kind) ?? 0) + 1;
    f.counts.set(kind, count);
    const call = { path, url, kind, id, attempt, page, count };
    f.calls.push(call);
    const override = await f.onRequest?.(call, f);
    if (override !== undefined) return clone(override.data);
    switch (kind) {
      case "review":
        return clone(f.review);
      case "pull":
        return clone(f.pull);
      case "workflow":
        return {
          workflow_runs: clone(
            f.workflowPages?.(page) ??
              f.runs.slice((page - 1) * 100, page * 100)
          )
        };
      case "detail": {
        const detail = f.details.has(id)
          ? f.details.get(id)
          : f.runs.find((run) => run.id === id);
        assert.ok(detail, `No fixture detail for run ${id}`);
        return clone(detail);
      }
      case "jobs": {
        const jobs =
          f.jobPages?.(id, attempt, page) ??
          (f.jobs.get(`${id}/${attempt}`) ?? []).slice(
            (page - 1) * 100,
            page * 100
          );
        return { jobs: clone(jobs) };
      }
      default:
        assert.fail(`Unhandled kind ${kind}`);
    }
  };
  f.wait = async (delay) => {
    f.delays.push(delay);
    f.clock += delay;
    await f.onWait?.(delay, f);
  };
  f.options = {
    repository: "vercel/turborepo",
    token: "test-token",
    runId: 200,
    pullNumber: 42,
    headSha: "head-sha",
    request: f.request,
    wait: f.wait
  };
  f.validate = (overrides = {}) =>
    validateReviewCI({ ...f.options, ...overrides });
  f.addLatest = (overrides = {}) => {
    const run = codeRun({ id: 101, run_number: 12, ...overrides });
    f.runs.push(run);
    f.jobs.set(`${run.id}/${run.run_attempt}`, [aggregate()]);
    return run;
  };
  return f;
}

function assertNoOldJobs(f) {
  assert.equal(
    f.calls.some((call) => call.kind === "jobs" && call.id === 100),
    false
  );
}

const RUN_MISMATCH = /CI run does not match/;
const PULL_MISMATCH = /Pull request revision changed or is not eligible/;
const BAD_AGGREGATE = /exactly one completed, successful aggregate/;
const TIMED_OUT = /Timed out waiting for code checks/;

for (const codeEmpty of [false, true]) {
  for (const reviewEmpty of [false, true]) {
    test(`accepts fork CI with code associations ${codeEmpty ? "empty" : "present"} and review associations ${reviewEmpty ? "empty" : "present"}`, async (t) => {
      const f = fixture(t);
      if (codeEmpty) f.runs[0].pull_requests = [];
      if (reviewEmpty) f.review.pull_requests = [];
      assert.equal(await f.validate(), 100);
      assert.deepEqual(f.delays, []);
      assert.deepEqual(
        f.calls.map((call) => call.kind),
        [
          "review",
          "pull",
          "workflow",
          "detail",
          "jobs",
          "workflow",
          "detail",
          "pull"
        ]
      );
      assert.equal(
        f.calls.find((call) => call.kind === "jobs").path,
        `${ROOT}/actions/runs/100/attempts/1/jobs?per_page=100&page=1`
      );
    });
  }
}

for (const empty of [false, true]) {
  for (const label of [
    "Code checks",
    "Code checks (PR #43, main)",
    "Code checks (PR #42, release)",
    "Code checks (PR #0, main)",
    `${AGGREGATE} suffix`
  ]) {
    test(`rejects aggregate label ${JSON.stringify(label)} with ${empty ? "empty" : "present"} associations`, async (t) => {
      const f = fixture(t);
      if (empty) f.runs[0].pull_requests = [];
      f.jobs.set("100/1", [{ ...aggregate(), name: label }]);
      await assert.rejects(f.validate(), BAD_AGGREGATE);
    });
  }
}

test("newer empty-associated run with wrong aggregate never falls back", async (t) => {
  const f = fixture(t);
  f.addLatest({ pull_requests: [] });
  f.jobs.set("101/1", [{ ...aggregate(), name: "Code checks (PR #43, main)" }]);
  await assert.rejects(f.validate(), BAD_AGGREGATE);
  assertNoOldJobs(f);
});

test("selects latest successful empty-associated run, not older success", async (t) => {
  const f = fixture(t);
  f.addLatest({ pull_requests: [] });
  assert.equal(await f.validate(), 101);
  assertNoOldJobs(f);
});

test("rejects duplicate aggregates for an empty-associated latest run", async (t) => {
  const f = fixture(t);
  f.addLatest({ pull_requests: [] });
  f.jobs.set("101/1", [aggregate(), aggregate()]);
  await assert.rejects(f.validate(), BAD_AGGREGATE);
  assertNoOldJobs(f);
});

test("waits for a queued empty-associated latest run", async (t) => {
  const f = fixture(t);
  const latest = f.addLatest({
    pull_requests: [],
    status: "queued",
    conclusion: null
  });
  f.jobs.set("101/1", []);
  f.onWait = () => {
    Object.assign(latest, { status: "completed", conclusion: "success" });
    f.jobs.set("101/1", [aggregate()]);
  };
  assert.equal(await f.validate(), 101);
  assert.deepEqual(f.delays, [15_000]);
  assertNoOldJobs(f);
});

test("accepts overall failure when the code aggregate succeeded", async (t) => {
  const f = fixture(t);
  f.runs[0].conclusion = "failure";
  assert.equal(await f.validate(), 100);
});

for (const conclusion of [
  "failure",
  "cancelled",
  "timed_out",
  "action_required",
  "neutral",
  "skipped",
  "stale",
  "startup_failure",
  null,
  undefined,
  "unknown"
]) {
  test(`rejects aggregate conclusion ${String(conclusion)}`, async (t) => {
    const f = fixture(t);
    f.jobs.set("100/1", [{ ...aggregate(), conclusion }]);
    await assert.rejects(f.validate(), BAD_AGGREGATE);
  });
}

for (const [name, jobs] of [
  ["missing", []],
  ["duplicate", [aggregate(), aggregate()]],
  ["in progress", [{ ...aggregate(), status: "in_progress" }]],
  ["queued", [{ ...aggregate(), status: "queued" }]],
  ["missing status", [{ ...aggregate(), status: undefined }]]
]) {
  test(`rejects ${name} aggregate`, async (t) => {
    const f = fixture(t);
    f.jobs.set("100/1", jobs);
    await assert.rejects(f.validate(), BAD_AGGREGATE);
  });
}

for (const conclusion of [
  "cancelled",
  "timed_out",
  "action_required",
  "neutral",
  "skipped",
  "stale",
  "startup_failure",
  null,
  undefined,
  "unknown"
]) {
  test(`rejects completed run conclusion ${String(conclusion)} without checking jobs`, async (t) => {
    const f = fixture(t);
    f.runs[0].conclusion = conclusion;
    await assert.rejects(f.validate(), /unacceptable conclusion/);
    assert.equal(f.counts.get("jobs"), undefined);
  });
}

test("latest failed code aggregate cannot fall back to older success", async (t) => {
  const f = fixture(t);
  f.addLatest({ conclusion: "failure" });
  f.jobs.set("101/1", [{ ...aggregate(), conclusion: "failure" }]);
  await assert.rejects(f.validate(), BAD_AGGREGATE);
  assertNoOldJobs(f);
});

for (const reverse of [false, true]) {
  test(`equal run numbers select greater id regardless of listing order (${reverse})`, async (t) => {
    const f = fixture(t);
    f.addLatest({ run_number: 10 });
    if (reverse) f.runs.reverse();
    assert.equal(await f.validate(), 101);
    assertNoOldJobs(f);
  });
}

test("run number takes precedence over a greater id", async (t) => {
  const f = fixture(t);
  f.addLatest({ id: 999, run_number: 9 });
  assert.equal(await f.validate(), 100);
});

test("ignores another PR with the same SHA and newer run number", async (t) => {
  const f = fixture(t);
  const other = association();
  other.number = 43;
  f.addLatest({ pull_requests: [other], conclusion: "cancelled" });
  assert.equal(await f.validate(), 100);
  assert.equal(
    f.calls.some((call) => call.id === 101),
    false
  );
});

for (const status of [
  "queued",
  "in_progress",
  "waiting",
  "pending",
  "requested"
]) {
  test(`polls ${status} latest run at 15 seconds, then accepts completion`, async (t) => {
    const f = fixture(t);
    f.runs[0].status = status;
    f.runs[0].conclusion = null;
    f.jobs.set("100/1", [{ ...aggregate(), status: "in_progress", conclusion: null }]);
    f.onWait = () => {
      Object.assign(f.runs[0], { status: "completed", conclusion: "success" });
      f.jobs.set("100/1", [aggregate()]);
    };
    assert.equal(await f.validate(), 100);
    assert.deepEqual(f.delays, [15_000]);
    assert.equal(f.counts.get("pull"), 3);
    assert.equal(f.counts.get("jobs"), 2);
  });
}

for (const status of [
  "queued",
  "in_progress",
  "waiting",
  "pending",
  "requested"
]) {
  test(`accepts a green aggregate from ${status} CI without waiting for cleanup`, async (t) => {
    const f = fixture(t);
    Object.assign(f.runs[0], { status, conclusion: null, run_attempt: 2 });
    f.jobs.set("100/2", [
      aggregate(),
      { name: "Cleanup", status: "in_progress", conclusion: null }
    ]);
    assert.equal(await f.validate({ maxAttempts: 1 }), 100);
    assert.deepEqual(f.delays, []);
    assert.equal(f.counts.get("workflow"), 2);
    assert.equal(f.counts.get("detail"), 2);
    assert.equal(f.counts.get("pull"), 2);
    assert.equal(f.calls.find((call) => call.kind === "jobs").attempt, 2);
  });
}

for (const [name, jobs] of [
  ["missing", []],
  ...["queued", "in_progress", "waiting", "pending", "requested"].map((status) => [
    status,
    [{ ...aggregate(), status, conclusion: null }]
  ])
]) {
  test(`polls ${name} aggregate until it succeeds while CI remains active`, async (t) => {
    const f = fixture(t);
    Object.assign(f.runs[0], { status: "in_progress", conclusion: null });
    f.jobs.set("100/1", jobs);
    f.onWait = () => f.jobs.set("100/1", [aggregate()]);
    assert.equal(await f.validate({ maxAttempts: 2 }), 100);
    assert.deepEqual(f.delays, [15_000]);
    assert.equal(f.counts.get("jobs"), 2);
    assert.equal(f.counts.get("pull"), 3);
    assert.equal(f.runs[0].status, "in_progress");
  });
}

for (const [name, jobs] of [
  ["duplicate", [aggregate(), aggregate()]],
  ["duplicate pending", [aggregate(), { ...aggregate(), status: "queued" }]],
  ...["failure", "cancelled", "timed_out", "neutral", "skipped", null, undefined].map(
    (conclusion) => [String(conclusion), [{ ...aggregate(), conclusion }]]
  )
]) {
  test(`rejects ${name} aggregate while CI is active`, async (t) => {
    const f = fixture(t);
    Object.assign(f.runs[0], { status: "in_progress", conclusion: null });
    f.jobs.set("100/1", jobs);
    await assert.rejects(f.validate(), BAD_AGGREGATE);
    assert.deepEqual(f.delays, []);
  });
}

for (const status of [undefined, null, "unknown", "cancelled", "failure", "success", "skipped", "neutral"]) {
  test(`rejects unexpected noncompleted run status ${String(status)}`, async (t) => {
    const f = fixture(t);
    f.runs[0].status = status;
    await assert.rejects(f.validate(), /Unexpected code CI run status/);
    assert.deepEqual(f.delays, []);
  });
}

test("discovers a newer successful run while waiting", async (t) => {
  const f = fixture(t);
  f.runs[0].status = "queued";
  f.jobs.set("100/1", []);
  f.onWait = () => f.addLatest();
  assert.equal(await f.validate(), 101);
  assert.deepEqual(f.delays, [15_000]);
  assert.deepEqual(
    f.calls.filter((call) => call.kind === "jobs").map((call) => call.id),
    [100, 101]
  );
});

test("a newer cancelled run appearing while waiting is not bypassed", async (t) => {
  const f = fixture(t);
  f.runs[0].status = "queued";
  f.jobs.set("100/1", []);
  f.onWait = () => f.addLatest({ conclusion: "cancelled" });
  await assert.rejects(f.validate(), /unacceptable conclusion/);
  assert.deepEqual(
    f.calls.filter((call) => call.kind === "jobs").map((call) => call.id),
    [100]
  );
});

test("rejects cancellation of the waiting run", async (t) => {
  const f = fixture(t);
  f.runs[0].status = "in_progress";
  f.jobs.set("100/1", []);
  f.onWait = () =>
    Object.assign(f.runs[0], { status: "completed", conclusion: "cancelled" });
  await assert.rejects(f.validate(), /unacceptable conclusion/);
  assert.deepEqual(f.delays, [15_000]);
});

for (const maxAttempts of [1, 2, 5, 120, undefined]) {
  test(`bounds polling with ${maxAttempts === undefined ? "default 120" : maxAttempts} attempts`, async (t) => {
    const f = fixture(t);
    f.runs[0].status = "queued";
    f.jobs.set("100/1", []);
    const bound = maxAttempts ?? 120;
    await assert.rejects(f.validate({ maxAttempts }), TIMED_OUT);
    assert.equal(f.counts.get("workflow"), bound);
    assert.equal(f.counts.get("detail"), bound);
    assert.equal(f.counts.get("pull"), bound);
    assert.deepEqual(f.delays, Array(bound - 1).fill(15_000));
    assert.equal(f.counts.get("jobs"), bound);
  });
}

test("can complete on the last permitted attempt", async (t) => {
  const f = fixture(t);
  f.runs[0].status = "queued";
  f.jobs.set("100/1", []);
  f.onWait = () => {
    if (f.delays.length === 2) {
      f.runs[0].status = "completed";
      f.jobs.set("100/1", [aggregate()]);
    }
  };
  assert.equal(await f.validate({ maxAttempts: 3 }), 100);
  assert.deepEqual(f.delays, [15_000, 15_000]);
});

test("wallclock timeout aborts before another request after a slow wait", async (t) => {
  const f = fixture(t);
  f.runs[0].status = "queued";
  f.jobs.set("100/1", []);
  f.onWait = () => {
    f.clock += TIMEOUT;
  };
  await assert.rejects(f.validate(), TIMED_OUT);
  assert.equal(f.counts.get("workflow"), 1);
  assert.equal(f.counts.get("pull"), 1);
});

test("wait delay is clamped to remaining wallclock budget", async (t) => {
  const f = fixture(t);
  f.runs[0].status = "queued";
  f.jobs.set("100/1", []);
  f.onRequest = ({ kind }) => {
    if (kind === "detail") f.clock += TIMEOUT - 7_000;
  };
  await assert.rejects(f.validate(), TIMED_OUT);
  assert.deepEqual(f.delays, [7_000]);
});

test("30-minute boundary is enforced after an API response", async (t) => {
  const f = fixture(t);
  f.onRequest = () => {
    f.clock += TIMEOUT;
  };
  await assert.rejects(f.validate(), TIMED_OUT);
  assert.equal(f.calls.length, 1);
});

// An associated latest candidate must be selected first, then rejected on any
// mismatch; a valid older candidate must never rescue it.
const runMutations = [
  [
    "workflow id",
    (run) => {
      run.workflow_id = 8;
    }
  ],
  [
    "workflow path",
    (run) => {
      run.path = ".github/workflows/other.yml";
    }
  ],
  [
    "workflow path suffix",
    (run) => {
      run.path += "@main";
    }
  ],
  [
    "event",
    (run) => {
      run.event = "pull_request_review";
    }
  ],
  [
    "head SHA",
    (run) => {
      run.head_sha = "different-sha";
    }
  ],
  [
    "head repository id",
    (run) => {
      run.head_repository.id = 3;
    }
  ],
  [
    "head repository name",
    (run) => {
      run.head_repository.full_name = "other/turborepo";
    }
  ],
  [
    "missing head repository",
    (run) => {
      delete run.head_repository;
    }
  ],
  [
    "head branch",
    (run) => {
      run.head_branch = "other-feature";
    }
  ],
  [
    "repository id",
    (run) => {
      run.repository.id = 3;
    }
  ],
  [
    "repository name",
    (run) => {
      run.repository.full_name = "other/turborepo";
    }
  ],
  [
    "missing repository",
    (run) => {
      delete run.repository;
    }
  ],
  [
    "association head SHA",
    (run) => {
      run.pull_requests[0].head.sha = "different-sha";
    }
  ],
  [
    "association head branch",
    (run) => {
      run.pull_requests[0].head.ref = "other-feature";
    }
  ],
  [
    "association head repo id",
    (run) => {
      run.pull_requests[0].head.repo.id = 3;
    }
  ],
  [
    "association base branch",
    (run) => {
      run.pull_requests[0].base.ref = "release";
    }
  ],
  [
    "association base repo id",
    (run) => {
      run.pull_requests[0].base.repo.id = 3;
    }
  ],
  [
    "missing association head",
    (run) => {
      delete run.pull_requests[0].head;
    }
  ],
  [
    "missing association head repo",
    (run) => {
      delete run.pull_requests[0].head.repo;
    }
  ],
  [
    "missing association base",
    (run) => {
      delete run.pull_requests[0].base;
    }
  ],
  [
    "missing association base repo",
    (run) => {
      delete run.pull_requests[0].base.repo;
    }
  ],
  [
    "duplicate PR association",
    (run) => {
      run.pull_requests.push(association());
    }
  ],
  [
    "run attempt zero",
    (run) => {
      run.run_attempt = 0;
    }
  ],
  [
    "run attempt missing",
    (run) => {
      delete run.run_attempt;
    }
  ],
  [
    "run attempt string",
    (run) => {
      run.run_attempt = "1";
    }
  ]
];

for (const [name, mutate] of runMutations) {
  test(`latest ${name} mismatch fails without older fallback`, async (t) => {
    const f = fixture(t);
    mutate(f.addLatest());
    await assert.rejects(f.validate(), RUN_MISMATCH);
    assertNoOldJobs(f);
    assert.equal(f.counts.get("jobs"), undefined);
  });
  test(`fresh code detail ${name} mismatch fails closed`, async (t) => {
    const f = fixture(t);
    const detail = clone(f.runs[0]);
    mutate(detail);
    f.details.set(100, detail);
    await assert.rejects(f.validate(), RUN_MISMATCH);
    assert.equal(f.counts.get("jobs"), undefined);
  });
  test(`review ${name} mismatch fails closed`, async (t) => {
    const f = fixture(t);
    // The initial review response supplies the expected workflow id itself.
    // A valid but different id is tested by the code-run mismatch above.
    if (name === "workflow id") f.review.workflow_id = 0;
    else if (name === "event") f.review.event = "pull_request";
    else mutate(f.review);
    await assert.rejects(
      f.validate(),
      name === "repository id"
        ? PULL_MISMATCH
        : /unexpected workflow, head, or repository|CI run does not match/
    );
    assert.equal(f.counts.get("workflow"), undefined);
  });
}

for (const [name, mutate] of runMutations.filter(([name]) =>
  [
    "head SHA",
    "head repository id",
    "head repository name",
    "missing head repository",
    "head branch"
  ].includes(name)
)) {
  test(`empty-associated ${name} mismatch is not a candidate`, async (t) => {
    const f = fixture(t);
    const latest = f.addLatest({ pull_requests: [] });
    mutate(latest);
    assert.equal(await f.validate(), 100);
    assert.equal(
      f.calls.some((call) => call.id === 101),
      false
    );
  });
}

for (const [field, value] of [
  ["id", 0],
  ["id", "101"],
  ["id", Number.MAX_SAFE_INTEGER + 1],
  ["run_number", 0],
  ["run_number", undefined],
  ["run_number", "12"]
]) {
  test(`invalid candidate ${field} ${String(value)} fails without fallback`, async (t) => {
    const f = fixture(t);
    f.addLatest({ [field]: value });
    await assert.rejects(f.validate(), /Invalid workflow run identity/);
    assertNoOldJobs(f);
  });
}

for (const [field, value] of [
  ["id", 101],
  ["run_number", 12]
]) {
  test(`fresh detail changes ${field} and is rejected`, async (t) => {
    const f = fixture(t);
    f.details.set(100, codeRun({ [field]: value }));
    await assert.rejects(f.validate(), /Unexpected code CI run identity/);
  });
}

const malformedAssociations = [
  ["missing", undefined],
  ["null", null],
  ["object", {}],
  ["string", "42"],
  ["null entry", [null]],
  ["missing number", [{}]],
  ["zero", [{ number: 0 }]],
  ["negative", [{ number: -42 }]],
  ["string number", [{ number: "42" }]],
  ["unsafe number", [{ number: Number.MAX_SAFE_INTEGER + 1 }]],
  ["malformed unrelated entry", [association(), { number: 0 }]]
];
for (const [name, pulls] of malformedAssociations) {
  for (const target of ["list", "detail", "review"]) {
    test(`${target} ${name} associations fail closed`, async (t) => {
      const f = fixture(t);
      if (target === "list") f.runs[0].pull_requests = clone(pulls);
      else if (target === "review") f.review.pull_requests = clone(pulls);
      else f.details.set(100, codeRun({ pull_requests: clone(pulls) }));
      await assert.rejects(
        f.validate(),
        target === "list"
          ? /Cannot prove workflow run PR association/
          : RUN_MISMATCH
      );
      assert.equal(f.counts.get("jobs"), undefined);
    });
  }
}

test("malformed associations on another PR are not silently skipped", async (t) => {
  const f = fixture(t);
  f.addLatest({ pull_requests: [{ number: 43 }, null] });
  await assert.rejects(
    f.validate(),
    /Cannot prove workflow run PR association/
  );
});

test("accepts one exact association alongside a different valid PR number", async (t) => {
  const f = fixture(t);
  f.runs[0].pull_requests.push({ number: 43 });
  assert.equal(await f.validate(), 100);
});

const pullMutations = [
  [
    "closed state",
    (pull) => {
      pull.state = "closed";
    }
  ],
  [
    "missing state",
    (pull) => {
      delete pull.state;
    }
  ],
  [
    "draft",
    (pull) => {
      pull.draft = true;
    }
  ],
  [
    "missing draft",
    (pull) => {
      delete pull.draft;
    }
  ],
  [
    "wrong number",
    (pull) => {
      pull.number = 43;
    }
  ],
  [
    "string number",
    (pull) => {
      pull.number = "42";
    }
  ],
  [
    "head SHA",
    (pull) => {
      pull.head.sha = "new-head";
    }
  ],
  [
    "head repo id",
    (pull) => {
      pull.head.repo.id = 0;
    }
  ],
  [
    "head repo name",
    (pull) => {
      pull.head.repo.full_name = "";
    }
  ],
  [
    "head branch",
    (pull) => {
      pull.head.ref = "";
    }
  ],
  [
    "missing head",
    (pull) => {
      delete pull.head;
    }
  ],
  [
    "missing head repo",
    (pull) => {
      delete pull.head.repo;
    }
  ],
  [
    "base repo id",
    (pull) => {
      pull.base.repo.id = 2;
    }
  ],
  [
    "base repo name",
    (pull) => {
      pull.base.repo.full_name = "other/turborepo";
    }
  ],
  [
    "base branch",
    (pull) => {
      pull.base.ref = "release";
    }
  ],
  [
    "empty base branch",
    (pull) => {
      pull.base.ref = "";
    }
  ],
  [
    "default branch",
    (pull) => {
      pull.base.repo.default_branch = "release";
    }
  ],
  [
    "missing default branch",
    (pull) => {
      delete pull.base.repo.default_branch;
    }
  ],
  [
    "missing base",
    (pull) => {
      delete pull.base;
    }
  ],
  [
    "missing base repo",
    (pull) => {
      delete pull.base.repo;
    }
  ]
];
for (const [name, mutate] of pullMutations) {
  test(`live PR ${name} is ineligible`, async (t) => {
    const f = fixture(t);
    mutate(f.pull);
    await assert.rejects(f.validate(), PULL_MISMATCH);
    assert.equal(f.counts.get("workflow"), undefined);
  });
}

const changedPullMutations = [
  ...pullMutations,
  [
    "different valid head repo id",
    (pull) => {
      pull.head.repo.id = 3;
    }
  ],
  [
    "different valid head repo name",
    (pull) => {
      pull.head.repo.full_name = "other/turborepo";
    }
  ],
  [
    "different valid head branch",
    (pull) => {
      pull.head.ref = "new-feature";
    }
  ],
  [
    "base and default branch move together",
    (pull) => {
      pull.base.ref = pull.base.repo.default_branch = "release";
    }
  ]
];
for (const [name, mutate] of changedPullMutations) {
  for (const phase of ["jobs", "waiting"]) {
    test(`rejects PR ${name} changed during ${phase}`, async (t) => {
      const f = fixture(t);
      if (phase === "jobs") {
        f.onRequest = ({ kind }) => {
          if (kind === "jobs") mutate(f.pull);
        };
      } else {
        f.runs[0].status = "queued";
        f.jobs.set("100/1", []);
        f.onWait = () => {
          mutate(f.pull);
          f.runs[0].status = "completed";
        };
      }
      await assert.rejects(f.validate(), PULL_MISMATCH);
      if (phase === "waiting") assert.equal(f.counts.get("jobs"), 1);
    });
  }
}

for (const phase of ["jobs", "waiting"]) {
  test(`permits base SHA advancement during ${phase}`, async (t) => {
    const f = fixture(t);
    const advance = () => {
      f.pull.base.sha = "advanced-base-sha";
    };
    if (phase === "jobs")
      f.onRequest = ({ kind }) => {
        if (kind === "jobs") advance();
      };
    else {
      f.runs[0].status = "queued";
      f.jobs.set("100/1", []);
      f.onWait = () => {
        advance();
        f.runs[0].status = "completed";
        f.jobs.set("100/1", [aggregate()]);
      };
    }
    assert.equal(await f.validate(), 100);
  });
}

for (const [name, mutate] of [
  ["new latest run", (f) => f.addLatest()],
  [
    "new attempt",
    (f) => {
      f.runs[0].run_attempt = 2;
    }
  ],
  [
    "queued rerun",
    (f) => {
      f.runs[0].status = "queued";
      f.runs[0].run_attempt = 2;
    }
  ],
  [
    "cancelled run",
    (f) => {
      Object.assign(f.runs[0], { status: "completed", conclusion: "cancelled" });
    }
  ],
  ["run no longer visible", (f) => { f.runs = []; }],
  ["invalid status", (f) => { f.runs[0].status = "unknown"; }]
]) {
  for (const status of ["completed", "in_progress"]) {
    test(`rejects ${name} race during ${status} job validation`, async (t) => {
      const f = fixture(t);
      f.runs[0].status = status;
      if (status === "in_progress") f.runs[0].conclusion = null;
      f.onRequest = ({ kind }) => {
        if (kind === "jobs") mutate(f);
      };
      await assert.rejects(
        f.validate(),
        /Latest code CI run or attempt changed during validation/
      );
      assert.equal(f.counts.get("jobs"), 1);
      assert.deepEqual(f.delays, []);
    });
  }
}

for (const [status, conclusion] of [
  ["queued", null],
  ["completed", "success"],
  ["completed", "failure"]
]) {
  test(`accepts same-attempt transition to ${status}/${conclusion} during aggregate validation`, async (t) => {
    const f = fixture(t);
    Object.assign(f.runs[0], { status: "in_progress", conclusion: null });
    f.onRequest = ({ kind }) => {
      if (kind === "jobs") Object.assign(f.runs[0], { status, conclusion });
    };
    assert.equal(await f.validate(), 100);
    assert.deepEqual(f.delays, []);
    assert.equal(f.counts.get("pull"), 2);
  });
}

for (const conclusion of ["timed_out", "action_required", "neutral", "skipped", "stale", "startup_failure", null, undefined, "unknown"]) {
  test(`rejects final completed conclusion ${String(conclusion)} after an active green aggregate`, async (t) => {
    const f = fixture(t);
    Object.assign(f.runs[0], { status: "in_progress", conclusion: null });
    f.onRequest = ({ kind }) => {
      if (kind === "jobs") Object.assign(f.runs[0], { status: "completed", conclusion });
    };
    await assert.rejects(f.validate(), /Latest code CI run or attempt changed/);
    assert.deepEqual(f.delays, []);
  });
}

test("active green aggregate still checks the final PR revision", async (t) => {
  const f = fixture(t);
  Object.assign(f.runs[0], { status: "in_progress", conclusion: null });
  f.onRequest = ({ kind }) => {
    if (kind === "jobs") f.pull.head.sha = "new-head";
  };
  await assert.rejects(f.validate(), PULL_MISMATCH);
  assert.equal(f.counts.get("pull"), 2);
});

test("active green aggregate still enforces the deadline after the final PR check", async (t) => {
  const f = fixture(t);
  Object.assign(f.runs[0], { status: "in_progress", conclusion: null });
  f.onRequest = ({ kind, count }) => {
    if (kind === "pull" && count === 2) f.clock += TIMEOUT;
  };
  await assert.rejects(f.validate(), TIMED_OUT);
  assert.equal(f.counts.get("pull"), 2);
});

test("attempt-specific jobs cannot reuse success from an older attempt", async (t) => {
  const f = fixture(t);
  f.runs[0].run_attempt = 2;
  f.jobs.set("100/2", [{ ...aggregate(), conclusion: "failure" }]);
  await assert.rejects(f.validate(), BAD_AGGREGATE);
  assert.deepEqual(
    f.calls.filter((call) => call.kind === "jobs").map((call) => call.attempt),
    [2]
  );
});

test("missing current attempt jobs cannot reuse older attempt success", async (t) => {
  const f = fixture(t);
  f.runs[0].run_attempt = 2;
  await assert.rejects(f.validate(), BAD_AGGREGATE);
  assert.equal(f.calls.find((call) => call.kind === "jobs").attempt, 2);
});

test("fresh detail attempt supersedes stale list attempt", async (t) => {
  const f = fixture(t);
  f.details.set(100, codeRun({ run_attempt: 2 }));
  f.jobs.set("100/2", [aggregate()]);
  assert.equal(await f.validate(), 100);
  assert.deepEqual(
    f.calls.filter((call) => call.kind === "jobs").map((call) => call.attempt),
    [2]
  );
});

function unrelatedRuns(count) {
  return Array.from({ length: count }, (_, index) =>
    codeRun({
      id: 1_000 + index,
      run_number: 1_000 + index,
      pull_requests: [{ number: 43 }]
    })
  );
}

function otherJobs(count) {
  return Array.from({ length: count }, (_, index) => ({
    name: `Other job ${index}`,
    status: "completed",
    conclusion: "failure"
  }));
}

test("paginates workflow runs and finds the latest on a later page", async (t) => {
  const f = fixture(t);
  const latest = f.addLatest();
  f.runs = [...unrelatedRuns(99), codeRun(), latest];
  assert.equal(await f.validate(), 101);
  assert.deepEqual(
    f.calls.filter((call) => call.kind === "workflow").map((call) => call.page),
    [1, 2, 1, 2]
  );
  assertNoOldJobs(f);
});

test("exactly 100 workflow results request an empty second page", async (t) => {
  const f = fixture(t);
  f.runs = [codeRun(), ...unrelatedRuns(99)];
  assert.equal(await f.validate(), 100);
  assert.deepEqual(
    f.calls.filter((call) => call.kind === "workflow").map((call) => call.page),
    [1, 2, 1, 2]
  );
});

test("latest mismatch on a later workflow page never falls back", async (t) => {
  const f = fixture(t);
  const latest = f.addLatest({ path: ".github/workflows/other.yml" });
  f.runs = [codeRun(), ...unrelatedRuns(99), latest];
  await assert.rejects(f.validate(), RUN_MISMATCH);
  assertNoOldJobs(f);
});

test("1000 filtered workflow results fail closed at the API cap", async (t) => {
  const f = fixture(t);
  f.runs = [codeRun(), ...unrelatedRuns(999)];
  await assert.rejects(f.validate(), /Workflow run search exceeds API limit/);
  assert.deepEqual(
    f.calls.filter((call) => call.kind === "workflow").map((call) => call.page),
    Array.from({ length: 10 }, (_, i) => i + 1)
  );
  assert.equal(f.counts.get("detail"), undefined);
  assert.equal(f.counts.get("jobs"), undefined);
});

test("999 workflow results remain below the cap", async (t) => {
  const f = fixture(t);
  f.runs = [codeRun(), ...unrelatedRuns(998)];
  assert.equal(await f.validate(), 100);
  assert.equal(f.counts.get("workflow"), 20);
});

for (const runs of [
  [],
  unrelatedRuns(2),
  [codeRun({ pull_requests: [], head_sha: "other-sha" })]
]) {
  test(`times out with no associated code run (${runs.length} listed, ${runs[0]?.pull_requests.length ?? 0} associations)`, async (t) => {
    const f = fixture(t);
    f.runs = clone(runs);
    await assert.rejects(f.validate({ maxAttempts: 2 }), TIMED_OUT);
    assert.deepEqual(f.delays, [15_000]);
    assert.equal(f.counts.get("workflow"), 2);
    assert.equal(f.counts.get("detail"), undefined);
    assert.equal(f.counts.get("jobs"), undefined);
  });
}

test("polls until the exact head's code run becomes visible", async (t) => {
  const f = fixture(t);
  f.runs = [
    codeRun({
      id: 99,
      run_number: 9,
      pull_requests: [],
      head_sha: "old-head-sha"
    })
  ];
  f.onWait = () => {
    f.runs.push(codeRun({ status: "in_progress", conclusion: null }));
  };
  assert.equal(await f.validate({ maxAttempts: 2 }), 100);
  assert.deepEqual(f.delays, [15_000]);
  assert.equal(f.counts.get("workflow"), 3);
  assert.equal(f.counts.get("detail"), 2);
  assert.equal(f.counts.get("pull"), 3);
  assert.equal(f.calls.some((call) => call.id === 99), false);
});

for (const maxAttempts of [1, 5, 120, undefined]) {
  test(`missing code run times out within ${maxAttempts ?? "default 120"} attempts`, async (t) => {
    const f = fixture(t);
    f.runs = [];
    const bound = maxAttempts ?? 120;
    await assert.rejects(f.validate({ maxAttempts }), TIMED_OUT);
    assert.equal(f.counts.get("workflow"), bound);
    assert.equal(f.counts.get("pull"), bound);
    assert.equal(f.counts.get("detail"), undefined);
    assert.equal(f.counts.get("jobs"), undefined);
    assert.deepEqual(f.delays, Array(bound - 1).fill(15_000));
  });
}

test("missing code run polling respects the wallclock deadline", async (t) => {
  const f = fixture(t);
  f.runs = [];
  f.onRequest = ({ kind }) => {
    if (kind === "workflow") f.clock += TIMEOUT - 7_000;
  };
  await assert.rejects(f.validate(), TIMED_OUT);
  assert.deepEqual(f.delays, [7_000]);
  assert.equal(f.counts.get("workflow"), 1);
  assert.equal(f.counts.get("pull"), 1);
});

test("paginates jobs to find an aggregate on a later page", async (t) => {
  const f = fixture(t);
  f.jobs.set("100/1", [...otherJobs(100), aggregate()]);
  assert.equal(await f.validate(), 100);
  assert.deepEqual(
    f.calls.filter((call) => call.kind === "jobs").map((call) => call.page),
    [1, 2]
  );
});

test("duplicate aggregate on a later jobs page is rejected", async (t) => {
  const f = fixture(t);
  f.jobs.set("100/1", [aggregate(), ...otherJobs(99), aggregate()]);
  await assert.rejects(f.validate(), BAD_AGGREGATE);
  assert.deepEqual(
    f.calls.filter((call) => call.kind === "jobs").map((call) => call.page),
    [1, 2]
  );
});

test("exactly 100 jobs require checking an empty next page", async (t) => {
  const f = fixture(t);
  f.jobs.set("100/1", [aggregate(), ...otherJobs(99)]);
  assert.equal(await f.validate(), 100);
  assert.equal(f.counts.get("jobs"), 2);
});

test("job pagination has a 100-page safety limit", async (t) => {
  const f = fixture(t);
  f.jobPages = () => [aggregate(), ...otherJobs(99)];
  await assert.rejects(f.validate(), /Code CI job search exceeds safety limit/);
  assert.equal(f.counts.get("jobs"), 100);
  assert.equal(f.calls.filter((call) => call.kind === "jobs").at(-1).page, 100);
});

const endpointPhases = [
  ["initial review", "review", 1],
  ["initial PR", "pull", 1],
  ["workflow list", "workflow", 1],
  ["code detail", "detail", 1],
  ["attempt jobs", "jobs", 1],
  ["final workflow refetch", "workflow", 2],
  ["final detail refetch", "detail", 2],
  ["final PR refetch", "pull", 2]
];
for (const [name, kind, count] of endpointPhases) {
  test(`propagates API error from ${name}`, async (t) => {
    const f = fixture(t);
    const error = new Error(`API failed at ${name}`);
    f.onRequest = (call) => {
      if (call.kind === kind && call.count === count) throw error;
    };
    await assert.rejects(f.validate(), (actual) => actual === error);
    assert.equal(f.calls.at(-1).kind, kind);
    assert.equal(f.calls.at(-1).count, count);
  });
  test(`propagates malformed JSON from ${name}`, async (t) => {
    const f = fixture(t);
    const error = new SyntaxError(`Malformed JSON at ${name}`);
    f.onRequest = (call) => {
      if (call.kind === kind && call.count === count) throw error;
    };
    await assert.rejects(f.validate(), (actual) => actual === error);
  });
}

for (const [kind, error] of [
  ["workflow", /Missing workflow run results/],
  ["jobs", /Missing code CI job results/]
]) {
  for (const data of [
    undefined,
    null,
    {},
    { workflow_runs: {}, jobs: {} },
    { workflow_runs: null, jobs: null }
  ]) {
    test(`rejects malformed ${kind} payload ${JSON.stringify(data)}`, async (t) => {
      const f = fixture(t);
      f.onRequest = (call) => (call.kind === kind ? { data } : undefined);
      await assert.rejects(f.validate(), error);
    });
  }
}

for (const [name, kind, expected] of [
  ["review", "review", /Current review run has unexpected/],
  ["pull", "pull", PULL_MISMATCH],
  ["detail", "detail", RUN_MISMATCH]
]) {
  for (const data of [undefined, null, {}]) {
    test(`rejects malformed ${name} object ${JSON.stringify(data)}`, async (t) => {
      const f = fixture(t);
      f.onRequest = (call) => (call.kind === kind ? { data } : undefined);
      await assert.rejects(f.validate(), expected);
    });
  }
}

test("propagates wait errors without making another request", async (t) => {
  const f = fixture(t);
  f.runs[0].status = "queued";
  f.jobs.set("100/1", []);
  const error = new Error("wait failed");
  f.onWait = () => {
    throw error;
  };
  await assert.rejects(f.validate(), (actual) => actual === error);
  assert.equal(f.counts.get("workflow"), 1);
});

const initialReviewMutations = [
  [
    "id",
    (run) => {
      run.id = 201;
    }
  ],
  [
    "string id",
    (run) => {
      run.id = "200";
    }
  ],
  [
    "event",
    (run) => {
      run.event = "pull_request";
    }
  ],
  [
    "path",
    (run) => {
      run.path = ".github/workflows/other.yml";
    }
  ],
  [
    "missing workflow id",
    (run) => {
      delete run.workflow_id;
    }
  ],
  [
    "zero workflow id",
    (run) => {
      run.workflow_id = 0;
    }
  ],
  [
    "string workflow id",
    (run) => {
      run.workflow_id = "9";
    }
  ],
  [
    "unsafe workflow id",
    (run) => {
      run.workflow_id = Number.MAX_SAFE_INTEGER + 1;
    }
  ],
  [
    "zero repository id",
    (run) => {
      run.repository.id = 0;
    }
  ],
  [
    "string repository id",
    (run) => {
      run.repository.id = "1";
    }
  ],
  [
    "repository name",
    (run) => {
      run.repository.full_name = "other/turborepo";
    }
  ],
  [
    "missing repository",
    (run) => {
      delete run.repository;
    }
  ],
  [
    "head SHA",
    (run) => {
      run.head_sha = "other-sha";
    }
  ]
];
for (const [name, mutate] of initialReviewMutations) {
  test(`initial review ${name} mismatch stops before reading the PR`, async (t) => {
    const f = fixture(t);
    mutate(f.review);
    await assert.rejects(
      f.validate(),
      /Current review run has unexpected workflow, head, or repository/
    );
    assert.deepEqual(
      f.calls.map((call) => call.kind),
      ["review"]
    );
  });
}

for (const empty of [false, true]) {
  test(`review merge SHA is allowed with ${empty ? "empty" : "present"} associations`, async (t) => {
    const f = fixture(t);
    f.review.head_sha = "merge-sha";
    if (empty) f.review.pull_requests = [];
    assert.equal(await f.validate({ reviewSha: "merge-sha" }), 100);
  });
  test(`code merge SHA is never accepted with ${empty ? "empty" : "present"} associations`, async (t) => {
    const f = fixture(t);
    f.review.head_sha = "merge-sha";
    f.runs[0].head_sha = "merge-sha";
    if (empty) f.runs[0].pull_requests = [];
    await assert.rejects(
      f.validate({ reviewSha: "merge-sha", maxAttempts: 2 }),
      empty ? TIMED_OUT : RUN_MISMATCH
    );
  });
}

test("review association must bind the PR head, not its merge SHA", async (t) => {
  const f = fixture(t);
  f.review.head_sha = "merge-sha";
  f.review.pull_requests[0].head.sha = "merge-sha";
  await assert.rejects(f.validate({ reviewSha: "merge-sha" }), RUN_MISMATCH);
});

test("explicit review SHA also allows a review run on the original PR head", async (t) => {
  const f = fixture(t);
  assert.equal(await f.validate({ reviewSha: "merge-sha" }), 100);
});

const invalidOptions = [
  [
    "repository",
    [
      undefined,
      null,
      "",
      "vercel",
      "vercel/turborepo/extra",
      "/turborepo",
      "vercel/turbo repo",
      "https://api.github.com/repos/vercel/turborepo",
      "vercel/turborepo?x=1"
    ]
  ],
  ["token", [undefined, null, "", 123, false]],
  [
    "runId",
    [
      undefined,
      null,
      0,
      -1,
      1.5,
      "200",
      NaN,
      Infinity,
      Number.MAX_SAFE_INTEGER + 1
    ]
  ],
  [
    "pullNumber",
    [
      undefined,
      null,
      0,
      -1,
      1.5,
      "42",
      NaN,
      Infinity,
      Number.MAX_SAFE_INTEGER + 1
    ]
  ],
  ["headSha", [undefined, null, "", 123, false]],
  ["reviewSha", [null, "", 123, false]],
  [
    "maxAttempts",
    [null, 0, -1, 1.5, "120", 121, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]
  ],
  ["request", [null, {}, "request", 123, false]],
  ["wait", [null, {}, "wait", 123, false]]
];
for (const [key, values] of invalidOptions) {
  for (const value of values) {
    test(`rejects invalid option ${key}=${String(value)}`, async (t) => {
      const f = fixture(t);
      await assert.rejects(
        f.validate({ [key]: value }),
        /Invalid review CI configuration/
      );
      assert.deepEqual(f.calls, []);
      assert.deepEqual(f.delays, []);
    });
  }
}

test("missing configuration is rejected", async () => {
  await assert.rejects(validateReviewCI(), /Invalid review CI configuration/);
});

// The default transport uses the same fixture; it never touches the network.
function installFetch(t, f, hook) {
  const calls = [];
  t.mock.method(globalThis, "fetch", async (input, init) => {
    const url = new URL(input);
    calls.push({ url, init });
    assert.equal(url.origin, "https://api.github.com");
    assert.equal(init.method, "GET");
    assert.equal(init.redirect, "error");
    assert.equal(init.headers.Accept, "application/vnd.github+json");
    assert.equal(init.headers.Authorization, "Bearer test-token");
    assert.equal(init.headers["X-GitHub-Api-Version"], "2022-11-28");
    assert.ok(init.signal instanceof AbortSignal);
    assert.equal(init.signal.aborted, false);
    assert.equal(init.body, undefined);
    const response = await hook?.(url, init, calls.length);
    if (response !== undefined) return response;
    const data = await f.request(`${url.pathname}${url.search}`);
    return { ok: true, status: 200, json: async () => data };
  });
  return calls;
}

test("default fetch is GET-only on fixed origin with auth, version, redirect policy and AbortSignal", async (t) => {
  const f = fixture(t);
  const attacker = "https://attacker.invalid/steal-token";
  for (const run of [f.review, ...f.runs]) {
    run.url = attacker;
    run.jobs_url = attacker;
    run.repository.url = attacker;
    run.head_repository.url = attacker;
    run.pull_requests[0].url = attacker;
  }
  f.pull.url = attacker;
  f.jobs.get("100/1")[0].url = attacker;
  const calls = installFetch(t, f);
  assert.equal(await f.validate({ request: undefined }), 100);
  assert.equal(calls.length, 8);
  assert.deepEqual(
    calls.map(({ url }) => `${url.pathname}${url.search}`),
    f.calls.map(({ path }) => path)
  );
  assert.equal(
    calls.some(({ url }) => url.href.includes("attacker")),
    false
  );
});

for (const status of [403, 404, 500]) {
  test(`default fetch propagates HTTP ${status} with the API path`, async (t) => {
    const f = fixture(t);
    let parsed = false;
    const calls = installFetch(t, f, () => ({
      ok: false,
      status,
      json: async () => {
        parsed = true;
        return {};
      }
    }));
    await assert.rejects(
      f.validate({ request: undefined }),
      new RegExp(
        `GitHub API request failed \\(${status}\\): ${ROOT}/actions/runs/200`
      )
    );
    assert.equal(calls.length, 1);
    assert.equal(parsed, false);
  });
}

test("default fetch propagates network failures unchanged", async (t) => {
  const f = fixture(t);
  const error = new TypeError("network unavailable");
  installFetch(t, f, () => {
    throw error;
  });
  await assert.rejects(
    f.validate({ request: undefined }),
    (actual) => actual === error
  );
});

test("default fetch propagates redirect rejection unchanged", async (t) => {
  const f = fixture(t);
  const error = new TypeError("redirect disallowed");
  installFetch(t, f, (_url, init) => {
    assert.equal(init.redirect, "error");
    throw error;
  });
  await assert.rejects(
    f.validate({ request: undefined }),
    (actual) => actual === error
  );
});

test("default fetch propagates JSON parsing failures unchanged", async (t) => {
  const f = fixture(t);
  const error = new SyntaxError("Unexpected token in JSON");
  installFetch(t, f, () => ({
    ok: true,
    status: 200,
    json: async () => {
      throw error;
    }
  }));
  await assert.rejects(
    f.validate({ request: undefined }),
    (actual) => actual === error
  );
});

test("default fetch AbortSignal timeout is capped at 30 seconds", async (t) => {
  const f = fixture(t);
  const delays = [];
  const original = AbortSignal.timeout;
  t.mock.method(AbortSignal, "timeout", (delay) => {
    delays.push(delay);
    return original.call(AbortSignal, delay);
  });
  installFetch(t, f);
  assert.equal(await f.validate({ request: undefined }), 100);
  assert.deepEqual(delays, Array(8).fill(30_000));
});

test("default fetch AbortSignal timeout respects remaining deadline", async (t) => {
  const f = fixture(t);
  const delays = [];
  const original = AbortSignal.timeout;
  t.mock.method(AbortSignal, "timeout", (delay) => {
    delays.push(delay);
    return original.call(AbortSignal, delay);
  });
  installFetch(t, f, (_url, _init, count) => {
    if (count === 1) f.clock += TIMEOUT - 5_000;
  });
  assert.equal(await f.validate({ request: undefined }), 100);
  assert.deepEqual(delays, [30_000, ...Array(7).fill(5_000)]);
});
