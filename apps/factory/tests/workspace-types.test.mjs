import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { Client, defaultMessageReducer } from "eve/client";

import {
  isWorkspaceRunning,
  activeWorkspaceTurnId,
  WORKSPACE_CHAT_TURN_POLICY,
  workspaceChatContent,
  workspaceImageSelectionError,
  workspaceImageUrl,
  readWorkspaceChatImages,
  hasConfirmedWorkspaceMessage,
  MAX_WORKSPACE_IMAGE_BYTES,
  latestWorkspaceFailure,
  removeConfirmedOptimisticMessages,
  workspaceStatusLabel
} from "../app/workspace-types.ts";

test("workspace status labels describe operator-visible state", () => {
  assert.equal(workspaceStatusLabel("idle"), "Ready");
  assert.equal(workspaceStatusLabel("running"), "Working");
  assert.equal(workspaceStatusLabel("pending"), "Working");
  assert.equal(workspaceStatusLabel("error"), "Error");
  assert.equal(isWorkspaceRunning("running"), true);
  assert.equal(isWorkspaceRunning("idle"), false);
});

test("projects actionable failure information from workspace events", () => {
  const events = [
    { type: "turn.started", data: { turnId: "turn_1" } },
    {
      type: "step.failed",
      data: {
        code: "gateway-auth",
        message: "Failed",
        details: {
          hint: "Refresh the AI Gateway credentials.",
          detail: "401 Unauthorized\nCaused by an expired token."
        }
      }
    },
    {
      type: "turn.failed",
      data: {
        code: "gateway-auth",
        message: "Failed",
        details: {
          hint: "Refresh the AI Gateway credentials.",
          detail: "401 Unauthorized\nCaused by an expired token."
        }
      }
    }
  ];
  assert.deepEqual(latestWorkspaceFailure(events), {
    code: "gateway-auth",
    message: "Failed",
    hint: "Refresh the AI Gateway credentials.",
    detail: "401 Unauthorized\nCaused by an expired token."
  });
});

test("clears an old workspace failure when a later turn starts", () => {
  assert.equal(
    latestWorkspaceFailure([
      {
        type: "session.failed",
        data: { code: "failed", message: "First run failed" }
      },
      { type: "turn.started", data: { turnId: "turn_2" } }
    ]),
    undefined
  );
});

test("ignores malformed workspace failure events", () => {
  assert.equal(
    latestWorkspaceFailure([
      { type: "turn.failed", data: { code: "failed", message: "  " } }
    ]),
    undefined
  );
});

function userMessage(text) {
  return { role: "user", parts: [{ type: "text", text }] };
}

test("optimistic confirmation preserves state identity when there is nothing to confirm", () => {
  const empty = [];
  assert.equal(removeConfirmedOptimisticMessages(empty, []), empty);
  const optimistic = [
    { id: "sent_1", text: "Fix cache", afterMessageCount: 1 }
  ];
  // A repeated older prompt and an assistant response do not acknowledge a optimistic entry.
  const messages = [
    userMessage("Fix cache"),
    { role: "assistant", parts: [{ type: "text", text: "Fix cache" }] }
  ];
  assert.equal(
    removeConfirmedOptimisticMessages(optimistic, messages),
    optimistic
  );
  // Even freshly rebuilt projections must not trigger another state update.
  for (let render = 0; render < 100; render++) {
    assert.equal(
      removeConfirmedOptimisticMessages(optimistic, structuredClone(messages)),
      optimistic
    );
  }
});

test("optimistic confirmation removes only confirmed messages and then stabilizes", () => {
  const optimistic = [
    { id: "sent_1", text: "Fix cache", afterMessageCount: 1 },
    { id: "sent_2", text: "Run tests", afterMessageCount: 1 }
  ];
  const messages = [
    userMessage("Original request"),
    userMessage("  Fix cache  ")
  ];
  const pending = removeConfirmedOptimisticMessages(optimistic, messages);
  assert.deepEqual(pending, [optimistic[1]]);
  assert.notEqual(pending, optimistic);
  assert.equal(pending[0], optimistic[1]);
  assert.equal(optimistic.length, 2);
  assert.equal(
    removeConfirmedOptimisticMessages(pending, structuredClone(messages)),
    pending
  );
  const empty = removeConfirmedOptimisticMessages(pending, [
    ...messages,
    userMessage("Run tests")
  ]);
  assert.deepEqual(empty, []);
  assert.equal(
    removeConfirmedOptimisticMessages(empty, structuredClone(messages)),
    empty
  );
});

test("optimistic confirmation combines text parts and ignores non-attachment parts", () => {
  const optimistic = [{ text: "Fix\ncache", afterMessageCount: 0 }];
  const messages = [
    {
      role: "user",
      parts: [
        { type: "text", text: "Fix" },
        { type: "reasoning", text: "ignored" },
        { type: "text", text: "cache" }
      ]
    }
  ];
  assert.deepEqual(removeConfirmedOptimisticMessages(optimistic, messages), []);
});

const image = {
  id: "image_1",
  name: "screenshot.png",
  mediaType: "image/png",
  size: 68,
  url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a8S8AAAAASUVORK5CYII="
};

test("chat serializes real image data for text-plus-image and image-only turns", () => {
  assert.equal(workspaceChatContent({ text: "Hello" }), "Hello");
  const file = {
    type: "file",
    data: image.url,
    mediaType: image.mediaType,
    filename: image.name
  };
  assert.deepEqual(
    workspaceChatContent({ text: "Inspect this", images: [image] }),
    [{ type: "text", text: "Inspect this" }, file]
  );
  assert.deepEqual(workspaceChatContent({ text: "", images: [image] }), [file]);
});

test("image selection enforces safe formats, count, and total request size", () => {
  const file = { type: image.mediaType, size: image.size };
  for (const type of ["image/png", "image/jpeg", "image/webp", "image/gif"])
    assert.equal(
      workspaceImageSelectionError([{ ...file, type }], []),
      undefined
    );
  for (const type of ["image/svg+xml", "text/html", "application/pdf", ""])
    assert.match(workspaceImageSelectionError([{ ...file, type }], []), /PNG/);
  assert.match(
    workspaceImageSelectionError([{ ...file, size: 0 }], []),
    /empty/
  );
  assert.match(
    workspaceImageSelectionError([file, file, file, file, file], []),
    /at most 4/
  );
  assert.match(
    workspaceImageSelectionError([file], [image, image, image, image]),
    /at most 4/
  );
  assert.equal(
    workspaceImageSelectionError(
      [{ ...file, size: MAX_WORKSPACE_IMAGE_BYTES }],
      []
    ),
    undefined
  );
  assert.match(
    workspaceImageSelectionError(
      [{ ...file, size: MAX_WORKSPACE_IMAGE_BYTES }],
      [image]
    ),
    /2 MiB/
  );
});

test("image previews reject executable or mismatched image URLs", () => {
  assert.equal(workspaceImageUrl("image/png", image.url), image.url);
  assert.equal(
    workspaceImageUrl("image/png", "https://example.com/image.png"),
    "https://example.com/image.png"
  );
  for (const url of [
    // oxlint-disable-next-line no-script-url -- Intentionally test rejection of executable URLs.
    "javascript:alert(1)",
    "data:text/html;base64,AAAA",
    "data:image/svg+xml;base64,AAAA",
    "http://example.com/image.png",
    "/image.png"
  ])
    assert.equal(workspaceImageUrl("image/png", url), undefined);
  assert.equal(
    workspaceImageUrl("image/svg+xml", "https://example.com/image.svg"),
    undefined
  );
  assert.equal(workspaceImageUrl("image/png"), undefined);
});

test("resumed Eve transcripts retain image URLs and confirm the matching image turn", () => {
  const reducer = defaultMessageReducer();
  const event = {
    type: "message.received",
    data: {
      turnId: "turn_image",
      message: "Inspect this",
      parts: [
        { type: "text", text: "Inspect this" },
        {
          type: "file",
          filename: image.name,
          mediaType: image.mediaType,
          url: image.url,
          size: image.size
        }
      ]
    }
  };
  const messages = reducer.reduce(
    reducer.initial(),
    // oxlint-disable-next-line unicorn/prefer-structured-clone -- Exercise JSON transcript persistence, not just cloning.
    JSON.parse(JSON.stringify(event))
  ).messages;
  const projectedImage = messages[0].parts[1];
  assert.equal(
    workspaceImageUrl(projectedImage.mediaType, projectedImage.url),
    image.url
  );
  const optimistic = [
    { text: "Inspect this", images: [image], afterMessageCount: 0 },
    {
      text: "Inspect this",
      images: [{ ...image, url: "data:image/png;base64,different" }],
      afterMessageCount: 0
    }
  ];
  assert.deepEqual(removeConfirmedOptimisticMessages(optimistic, messages), [
    optimistic[1]
  ]);
  assert.equal(hasConfirmedWorkspaceMessage(messages, optimistic[0]), true);
  assert.equal(hasConfirmedWorkspaceMessage(messages, optimistic[1]), false);
  assert.equal(
    hasConfirmedWorkspaceMessage(messages, {
      text: "Inspect this",
      afterMessageCount: 0
    }),
    false
  );
  const imageOnly = [{ role: "user", parts: [projectedImage] }];
  assert.equal(
    hasConfirmedWorkspaceMessage(imageOnly, {
      text: "",
      images: [image],
      afterMessageCount: 0
    }),
    true
  );
  assert.equal(
    hasConfirmedWorkspaceMessage(imageOnly, {
      text: "",
      images: [image],
      afterMessageCount: 1
    }),
    false
  );
});

test("browser image reading creates persistent data URLs and reports read failures", async (t) => {
  class Reader {
    readAsDataURL(file) {
      queueMicrotask(() => {
        if (file.name === "broken.png") this.onerror();
        else {
          this.result = image.url;
          this.onload();
        }
      });
    }
  }
  const original = globalThis.FileReader;
  globalThis.FileReader = Reader;
  t.after(() => {
    if (original === undefined) delete globalThis.FileReader;
    else globalThis.FileReader = original;
  });
  const files = [{ name: image.name, type: image.mediaType, size: image.size }];
  const images = await readWorkspaceChatImages(files, []);
  assert.equal(images[0].url, image.url);
  assert.equal(images[0].name, image.name);
  assert.ok(images[0].id);
  await assert.rejects(
    readWorkspaceChatImages([{ ...files[0], name: "broken.png" }], []),
    /Could not read/
  );
  await assert.rejects(
    readWorkspaceChatImages([{ ...files[0], type: "image/svg+xml" }], []),
    /PNG/
  );
});

test("Eve transport posts image bytes on normal and steering turns", async (t) => {
  const bodies = [];
  t.mock.method(globalThis, "fetch", async (url, init) => {
    assert.match(String(url), /session_image/);
    assert.equal(init.method, "POST");
    bodies.push(JSON.parse(init.body));
    return Response.json({ sessionId: "session_image" });
  });
  const session = new Client({
    host: "https://factory.example"
  }).sessions.attach("session_image");
  await session.send(
    workspaceChatContent({ text: "Inspect this", images: [image] }),
    { turnPolicy: WORKSPACE_CHAT_TURN_POLICY }
  );
  await session.send(workspaceChatContent({ text: "", images: [image] }), {
    turnPolicy: WORKSPACE_CHAT_TURN_POLICY,
    streamReconnectPolicy: { reconnect: false }
  });
  assert.equal(bodies[0].turnPolicy, "steer");
  assert.equal(bodies[0].message[1].data, image.url);
  assert.equal(bodies[0].message[1].mediaType, image.mediaType);
  assert.equal(bodies[1].message[0].data, image.url);
  assert.equal(bodies[1].turnPolicy, "steer");
});

test("repeated sent image messages consume distinct confirmations across cleanups", () => {
  const optimistic = [
    { id: "first", text: "", images: [image], afterMessageCount: 0 },
    { id: "second", text: "", images: [image], afterMessageCount: 0 }
  ];
  const confirmation = {
    role: "user",
    parts: [{ type: "file", mediaType: image.mediaType, url: image.url }]
  };
  const pending = removeConfirmedOptimisticMessages(optimistic, [confirmation]);
  assert.equal(pending.length, 1);
  assert.equal(pending[0].id, "second");
  assert.equal(
    removeConfirmedOptimisticMessages(pending, [confirmation]),
    pending
  );
  assert.deepEqual(
    removeConfirmedOptimisticMessages(pending, [confirmation, confirmation]),
    []
  );
});

test("chat sends every message directly with steering and has no queue UI", () => {
  const source = readFileSync(
    new URL("../app/workspace-client.tsx", import.meta.url),
    "utf8"
  );
  assert.equal(WORKSPACE_CHAT_TURN_POLICY, "steer");
  assert.match(source, /turnPolicy: WORKSPACE_CHAT_TURN_POLICY/);
  assert.doesNotMatch(
    source,
    /useEveAgent|QueuedMessages|queuedMessages|Enter to queue|Queue message|turnPolicy: "queue"/
  );
  assert.match(source, /\.send\(workspaceChatContent\(message\)/);
});

test("steered turns ignore stale completion and cancellation events", () => {
  const events = [
    { type: "turn.started", data: { turnId: "old" } },
    { type: "turn.started", data: { turnId: "replacement" } },
    { type: "turn.cancelled", data: { turnId: "old" } }
  ];
  assert.equal(activeWorkspaceTurnId(events), "replacement");
  assert.equal(
    activeWorkspaceTurnId([
      ...events,
      { type: "turn.completed", data: { turnId: "old" } }
    ]),
    "replacement"
  );
  assert.equal(
    activeWorkspaceTurnId([
      ...events,
      { type: "turn.completed", data: { turnId: "replacement" } }
    ]),
    undefined
  );
  assert.equal(
    activeWorkspaceTurnId([...events, { type: "session.failed" }]),
    undefined
  );
});
