/**
 * Parses a subset of the Mermaid sequence diagram DSL.
 *
 * Supported syntax:
 *   sequenceDiagram
 *   participant A as Alice
 *   actor B as Bob
 *   A->>B: Message         (solid arrow)
 *   A-->>B: Response        (dashed arrow)
 *   A-)B: Async message     (open arrow)
 *   A--)B: Async response   (dashed open arrow)
 *   A-xB: Lost message      (cross)
 *   A--xB: Lost response    (dashed cross)
 *   Note over A,B: text
 *   Note left of A: text
 *   Note right of A: text
 *   loop Title ... end
 *   alt Title ... else Title ... end
 *   opt Title ... end
 */

export type ArrowStyle =
  | "solid"
  | "dashed"
  | "solid-open"
  | "dashed-open"
  | "solid-cross"
  | "dashed-cross";

export interface SequenceParticipant {
  id: string;
  label: string;
  type: "participant" | "actor";
}

export interface SequenceMessage {
  kind: "message";
  from: string;
  to: string;
  text: string;
  arrowStyle: ArrowStyle;
}

export interface SequenceNote {
  kind: "note";
  position: "over" | "left" | "right";
  participants: string[];
  text: string;
}

export interface SequenceBlock {
  kind: "loop" | "alt" | "opt" | "else";
  label: string;
  children: SequenceEvent[];
}

export type SequenceEvent = SequenceMessage | SequenceNote | SequenceBlock;

export interface SequenceDiagram {
  participants: SequenceParticipant[];
  events: SequenceEvent[];
}

function parseArrowStyle(arrow: string): ArrowStyle {
  if (arrow === "->>") return "solid";
  if (arrow === "-->>") return "dashed";
  if (arrow === "-)") return "solid-open";
  if (arrow === "--)") return "dashed-open";
  if (arrow === "-x") return "solid-cross";
  if (arrow === "--x") return "dashed-cross";
  return "solid";
}

export function parseSequenceDiagram(input: string): SequenceDiagram {
  const lines = input
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  const participants: SequenceParticipant[] = [];
  const participantIds = new Set<string>();
  const eventStack: SequenceEvent[][] = [[]];

  let startIndex = 0;
  if (lines[0]?.toLowerCase() === "sequencediagram") {
    startIndex = 1;
  }

  function ensureParticipant(id: string): void {
    if (!participantIds.has(id)) {
      participantIds.add(id);
      participants.push({ id, label: id, type: "participant" });
    }
  }

  function currentEvents(): SequenceEvent[] {
    return eventStack[eventStack.length - 1];
  }

  for (let i = startIndex; i < lines.length; i++) {
    const line = lines[i];

    // Skip comments
    if (line.startsWith("%%")) continue;

    // Participant / actor declaration
    const participantMatch = line.match(
      /^(participant|actor)\s+(\S+)(?:\s+as\s+(.+))?$/i
    );
    if (participantMatch) {
      const type = participantMatch[1].toLowerCase() as "participant" | "actor";
      const id = participantMatch[2];
      const label = participantMatch[3] ?? id;
      if (!participantIds.has(id)) {
        participantIds.add(id);
        participants.push({ id, label, type });
      } else {
        const existing = participants.find((p) => p.id === id);
        if (existing && participantMatch[3]) {
          existing.label = label;
          existing.type = type;
        }
      }
      continue;
    }

    // Note
    const noteMatch = line.match(
      /^Note\s+(over|left of|right of)\s+([^:]+):\s*(.+)$/i
    );
    if (noteMatch) {
      const posRaw = noteMatch[1].toLowerCase();
      let position: SequenceNote["position"] = "over";
      if (posRaw === "left of") position = "left";
      else if (posRaw === "right of") position = "right";

      const participantRefs = noteMatch[2].split(",").map((s) => s.trim());
      for (const p of participantRefs) ensureParticipant(p);

      currentEvents().push({
        kind: "note",
        position,
        participants: participantRefs,
        text: noteMatch[3].trim()
      });
      continue;
    }

    // Block start: loop, alt, opt
    const blockMatch = line.match(/^(loop|alt|opt)\s+(.+)$/i);
    if (blockMatch) {
      const block: SequenceBlock = {
        kind: blockMatch[1].toLowerCase() as SequenceBlock["kind"],
        label: blockMatch[2],
        children: []
      };
      currentEvents().push(block);
      eventStack.push(block.children);
      continue;
    }

    // else inside alt
    const elseMatch = line.match(/^else(?:\s+(.+))?$/i);
    if (elseMatch) {
      // Pop from alt's current section, push an else block
      eventStack.pop();
      const parentEvents = currentEvents();
      const elseBlock: SequenceBlock = {
        kind: "else",
        label: elseMatch[1] ?? "",
        children: []
      };
      parentEvents.push(elseBlock);
      eventStack.push(elseBlock.children);
      continue;
    }

    // Block end
    if (line.toLowerCase() === "end") {
      if (eventStack.length > 1) {
        eventStack.pop();
      }
      continue;
    }

    // Message: A->>B: text  or  A-->>B: text  etc.
    const msgMatch = line.match(
      /^(\S+)\s*(--?>>|--?x|--?\))\s*(\S+)\s*:\s*(.+)$/
    );
    if (msgMatch) {
      const from = msgMatch[1];
      const arrow = msgMatch[2];
      const to = msgMatch[3];
      const text = msgMatch[4].trim();

      ensureParticipant(from);
      ensureParticipant(to);

      currentEvents().push({
        kind: "message",
        from,
        to,
        text,
        arrowStyle: parseArrowStyle(arrow)
      });
      continue;
    }
  }

  return { participants, events: eventStack[0] };
}
