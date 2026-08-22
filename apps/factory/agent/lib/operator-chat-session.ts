import type { ClientSessionState, MessageStreamEvent } from "eve/client";

/**
 * Serialization for the operator console's chat thread.
 *
 * A chat session is durable on the server, so a reload should land back in the
 * same conversation instead of orphaning a running sandbox. The console keeps
 * the session cursor and the rendered event log in browser storage and replays
 * both into `useEveAgent`. Long turns can produce megabytes of tool output, so
 * an oversized log is dropped and only the cursor survives: an empty event log
 * is still a valid ordered prefix of the session's stream.
 */

export const MAX_SAVED_CHAT_BYTES = 512_000;

export interface SavedChat {
  readonly events: readonly MessageStreamEvent[];
  readonly session: ClientSessionState;
}

interface ChatSnapshot {
  readonly events: readonly MessageStreamEvent[];
  readonly session: ClientSessionState | undefined;
}

function isSessionState(value: unknown): value is ClientSessionState {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.sessionId === "string" &&
    candidate.sessionId.length > 0 &&
    typeof candidate.streamIndex === "number" &&
    Number.isInteger(candidate.streamIndex) &&
    candidate.streamIndex >= 0
  );
}

/**
 * Renders a snapshot for storage, or `null` when there is no session worth
 * resuming.
 */
export function serializeSavedChat(snapshot: ChatSnapshot): string | null {
  if (!isSessionState(snapshot.session)) return null;
  const saved = JSON.stringify({
    events: snapshot.events,
    session: snapshot.session
  });
  return saved.length <= MAX_SAVED_CHAT_BYTES
    ? saved
    : JSON.stringify({ events: [], session: snapshot.session });
}

/** Reads a stored thread, ignoring anything an older format left behind. */
export function parseSavedChat(raw: string | null): SavedChat | null {
  if (raw === null) return null;
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return null;
  }

  if (typeof value !== "object" || value === null) return null;
  const candidate = value as Record<string, unknown>;
  if (!isSessionState(candidate.session)) return null;
  return {
    events: Array.isArray(candidate.events)
      ? (candidate.events as MessageStreamEvent[])
      : [],
    session: candidate.session
  };
}
