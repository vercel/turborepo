import assert from "node:assert/strict";
import test from "node:test";

import {
  createRemoteOpenCode,
  openCodeSessionID
} from "../agent/lib/remote-opencode-harness.ts";

function json(value, status = 200) {
  return new Response(JSON.stringify(value), {
    headers: { "content-type": "application/json" },
    status
  });
}

function eventStream(events) {
  return new Response(events.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(""), {
    headers: { "content-type": "text/event-stream" }
  });
}

function event(seq, type, data) {
  return { data, durable: { seq }, type };
}

test("maps a remote OpenCode turn to Harness stream parts", async () => {
  const requests = [];
  let created = false;
  const fetch = async (url, init = {}) => {
    requests.push({ body: init.body, method: init.method ?? "GET", url: String(url) });
    const path = new URL(url).pathname;
    if (path === "/api/session/ses_eve_operator") {
      return created
        ? json({ data: {
            id: "ses_eve_operator",
            location: { directory: "/workspace/projects/turborepo" },
            title: "[Eve] Daily example maintenance"
          } })
        : new Response(null, { status: 404 });
    }
    if (path === "/api/session") {
      created = true;
      return json({
        data: {
          id: "ses_eve_operator",
          location: { directory: "/workspace/projects/turborepo" },
          title: "[Eve] Daily example maintenance"
        }
      });
    }
    if (path.endsWith("/prompt") || path.endsWith("/wait")) return json({ data: {} });
    if (path.endsWith("/log")) {
      const prompt = requests.find((request) => request.url.endsWith("/prompt"));
      if (!prompt) return eventStream([]);
      const promptID = JSON.parse(prompt.body).id;
      return eventStream([
        event(2, "session.inbox.enqueued", { inboxID: promptID }),
        event(3, "session.text.ended", {
          assistantMessageID: "msg_assistant",
          ordinal: 0,
          text: "Updated the example."
        }),
        event(4, "session.step.ended", {
          files: ["examples/basic/package.json"],
          finish: "stop",
          tokens: { cache: { read: 2, write: 1 }, input: 10, output: 5, reasoning: 1 }
        }),
        event(5, "session.execution.succeeded", {})
      ]);
    }
    throw new Error(`Unexpected request: ${init.method ?? "GET"} ${url}`);
  };

  const harness = createRemoteOpenCode({
    baseURL: "https://opencode.test",
    fetch,
    location: { directory: "/workspace/projects/turborepo" },
    title: "[Eve] Daily example maintenance"
  });
  const session = await harness.doStart({
    sandboxSession: {},
    sessionId: "ses_eve_operator",
    sessionWorkDir: "/unused"
  });
  const emitted = [];
  const control = await session.doPromptTurn({
    emit: (part) => emitted.push(part),
    prompt: "Update the selected example."
  });
  await control.done;

  assert.equal(emitted.find((part) => part.type === "text-delta")?.delta, "Updated the example.");
  assert.equal(emitted.at(-1).type, "finish");
  assert.equal(JSON.parse(requests.find((request) => request.url.endsWith("/prompt")).body).id.length, 44);

  const replay = await harness.doStart({
    sandboxSession: {},
    sessionId: "ses_eve_operator",
    sessionWorkDir: "/unused"
  });
  const replayControl = await replay.doPromptTurn({
    emit: () => {},
    prompt: "Update the selected example."
  });
  await replayControl.done;
  assert.equal(requests.filter((request) => request.url.endsWith("/prompt")).length, 1);
});

test("rejects adopting a session from another workspace", async () => {
  const harness = createRemoteOpenCode({
    baseURL: "https://opencode.test",
    fetch: async () => json({
      data: {
        id: "ses_eve_operator",
        location: { directory: "/workspace/projects/other" }
      }
    }),
    location: { directory: "/workspace/projects/turborepo" }
  });

  await assert.rejects(
    harness.doStart({
      sandboxSession: {},
      sessionId: "ses_eve_operator",
      sessionWorkDir: "/unused"
    }),
    /does not match requested location/
  );
});

test("maps arbitrary harness IDs to stable OpenCode session IDs", () => {
  assert.equal(openCodeSessionID("ses_existing"), "ses_existing");
  assert.equal(openCodeSessionID("eve-run-1"), openCodeSessionID("eve-run-1"));
  assert.match(openCodeSessionID("eve-run-1"), /^ses_harness_[a-f0-9]{32}$/);
});
