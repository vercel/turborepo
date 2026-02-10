"use client";

import { useTheme } from "next-themes";
import { useEffect, useMemo, useState } from "react";
import type {
  ArrowStyle,
  SequenceDiagram as ParsedSequence,
  SequenceBlock,
  SequenceEvent,
  SequenceMessage,
  SequenceNote,
  SequenceParticipant
} from "./parse-sequence";

const PARTICIPANT_WIDTH = 120;
const PARTICIPANT_HEIGHT = 36;
const PARTICIPANT_GAP = 40;
const MESSAGE_HEIGHT = 40;
const NOTE_HEIGHT = 32;
const BLOCK_PADDING = 10;
const TOP_MARGIN = 20;
const BOTTOM_MARGIN = 40;
const LIFELINE_START = TOP_MARGIN + PARTICIPANT_HEIGHT + 10;

interface Colors {
  bg: string;
  text: string;
  border: string;
  lifeline: string;
  arrow: string;
  noteBg: string;
  blockBg: string;
}

function darkColors(): Colors {
  return {
    bg: "rgb(17, 17, 17)",
    text: "rgb(220, 220, 220)",
    border: "rgb(60, 60, 60)",
    lifeline: "rgb(50, 50, 50)",
    arrow: "rgb(150, 150, 150)",
    noteBg: "rgb(40, 40, 30)",
    blockBg: "rgba(60, 60, 60, 0.15)"
  };
}

function lightColors(): Colors {
  return {
    bg: "rgb(255, 255, 255)",
    text: "rgb(30, 30, 30)",
    border: "rgb(200, 200, 200)",
    lifeline: "rgb(220, 220, 220)",
    arrow: "rgb(100, 100, 100)",
    noteBg: "rgb(255, 255, 235)",
    blockBg: "rgba(200, 200, 200, 0.15)"
  };
}

function participantX(index: number): number {
  return index * (PARTICIPANT_WIDTH + PARTICIPANT_GAP) + PARTICIPANT_WIDTH / 2;
}

interface EventLayout {
  y: number;
  height: number;
}

function measureEvents(
  events: SequenceEvent[],
  startY: number
): { totalHeight: number; layouts: EventLayout[] } {
  let y = startY;
  const layouts: EventLayout[] = [];

  for (const event of events) {
    if (event.kind === "message") {
      layouts.push({ y, height: MESSAGE_HEIGHT });
      y += MESSAGE_HEIGHT;
    } else if (event.kind === "note") {
      layouts.push({ y, height: NOTE_HEIGHT });
      y += NOTE_HEIGHT;
    } else {
      // Block (loop/alt/opt/else)
      const headerHeight = 24;
      const childStart = y + headerHeight;
      const { totalHeight: childrenHeight } = measureEvents(
        event.children,
        childStart
      );
      const blockHeight = headerHeight + childrenHeight + BLOCK_PADDING;
      layouts.push({ y, height: blockHeight });
      y += blockHeight;
    }
  }

  return { totalHeight: y - startY, layouts };
}

function renderArrow(
  fromX: number,
  toX: number,
  y: number,
  style: ArrowStyle,
  colors: Colors
): React.ReactNode {
  const isDashed = style.startsWith("dashed");
  const isCross = style.endsWith("cross");
  const isOpen = style.endsWith("open");
  const dir = toX > fromX ? 1 : -1;
  const arrowSize = 6;

  return (
    <g key={`arrow-${fromX}-${toX}-${y}`}>
      <line
        x1={fromX}
        y1={y}
        x2={toX}
        y2={y}
        stroke={colors.arrow}
        strokeWidth={1.5}
        strokeDasharray={isDashed ? "6 3" : undefined}
      />
      {isCross ? (
        <>
          <line
            x1={toX - dir * arrowSize}
            y1={y - arrowSize}
            x2={toX + dir * arrowSize}
            y2={y + arrowSize}
            stroke={colors.arrow}
            strokeWidth={1.5}
          />
          <line
            x1={toX + dir * arrowSize}
            y1={y - arrowSize}
            x2={toX - dir * arrowSize}
            y2={y + arrowSize}
            stroke={colors.arrow}
            strokeWidth={1.5}
          />
        </>
      ) : isOpen ? (
        <>
          <line
            x1={toX - dir * arrowSize}
            y1={y - arrowSize}
            x2={toX}
            y2={y}
            stroke={colors.arrow}
            strokeWidth={1.5}
          />
          <line
            x1={toX - dir * arrowSize}
            y1={y + arrowSize}
            x2={toX}
            y2={y}
            stroke={colors.arrow}
            strokeWidth={1.5}
          />
        </>
      ) : (
        <polygon
          points={`${toX},${y} ${toX - dir * arrowSize},${y - arrowSize} ${toX - dir * arrowSize},${y + arrowSize}`}
          fill={colors.arrow}
        />
      )}
    </g>
  );
}

function renderEvents(
  events: SequenceEvent[],
  layouts: EventLayout[],
  participantIndex: Map<string, number>,
  colors: Colors,
  keyPrefix: string
): React.ReactNode[] {
  const elements: React.ReactNode[] = [];

  for (let i = 0; i < events.length; i++) {
    const event = events[i];
    const { y } = layouts[i];

    if (event.kind === "message") {
      const fromIdx = participantIndex.get(event.from) ?? 0;
      const toIdx = participantIndex.get(event.to) ?? 0;
      const fromX = participantX(fromIdx);
      const toX = participantX(toIdx);
      const arrowY = y + MESSAGE_HEIGHT / 2;

      elements.push(renderArrow(fromX, toX, arrowY, event.arrowStyle, colors));

      // Message text
      const midX = (fromX + toX) / 2;
      elements.push(
        <text
          key={`${keyPrefix}-msg-${i}`}
          x={midX}
          y={arrowY - 8}
          textAnchor="middle"
          fill={colors.text}
          fontSize={11}
          fontFamily="var(--font-mono, monospace)"
        >
          {event.text}
        </text>
      );
    } else if (event.kind === "note") {
      const indices = event.participants.map(
        (p) => participantIndex.get(p) ?? 0
      );
      const minIdx = Math.min(...indices);
      const maxIdx = Math.max(...indices);
      const x1 = participantX(minIdx) - 40;
      const x2 = participantX(maxIdx) + 40;
      const noteY = y + 4;

      elements.push(
        <g key={`${keyPrefix}-note-${i}`}>
          <rect
            x={x1}
            y={noteY}
            width={x2 - x1}
            height={NOTE_HEIGHT - 8}
            fill={colors.noteBg}
            stroke={colors.border}
            strokeWidth={1}
            rx={3}
          />
          <text
            x={(x1 + x2) / 2}
            y={noteY + (NOTE_HEIGHT - 8) / 2 + 4}
            textAnchor="middle"
            fill={colors.text}
            fontSize={11}
            fontFamily="var(--font-mono, monospace)"
          >
            {event.text}
          </text>
        </g>
      );
    } else {
      // Block: loop, alt, opt, else
      const blockHeight = layouts[i].height;
      const allParticipantIndices = Array.from(participantIndex.values());
      const minIdx = Math.min(...allParticipantIndices);
      const maxIdx = Math.max(...allParticipantIndices);
      const x1 = participantX(minIdx) - PARTICIPANT_WIDTH / 2 - 10;
      const x2 = participantX(maxIdx) + PARTICIPANT_WIDTH / 2 + 10;

      elements.push(
        <g key={`${keyPrefix}-block-${i}`}>
          <rect
            x={x1}
            y={y}
            width={x2 - x1}
            height={blockHeight}
            fill={colors.blockBg}
            stroke={colors.border}
            strokeWidth={1}
            strokeDasharray="4 2"
            rx={4}
          />
          <text
            x={x1 + 8}
            y={y + 16}
            fill={colors.text}
            fontSize={10}
            fontWeight={600}
            fontFamily="var(--font-mono, monospace)"
          >
            {event.kind.toUpperCase()}
            {event.label ? ` [${event.label}]` : ""}
          </text>
        </g>
      );

      // Render children
      const headerHeight = 24;
      const { layouts: childLayouts } = measureEvents(
        event.children,
        y + headerHeight
      );
      elements.push(
        ...renderEvents(
          event.children,
          childLayouts,
          participantIndex,
          colors,
          `${keyPrefix}-block-${i}`
        )
      );
    }
  }

  return elements;
}

interface SequenceDiagramProps {
  diagram: ParsedSequence;
}

export function SequenceDiagram({ diagram }: SequenceDiagramProps) {
  const [mounted, setMounted] = useState(false);
  const { resolvedTheme } = useTheme();

  useEffect(() => {
    setMounted(true);
  }, []);

  const rendered = useMemo(() => {
    const colors = resolvedTheme === "dark" ? darkColors() : lightColors();
    const { participants, events } = diagram;

    const participantIndex = new Map<string, number>();
    participants.forEach((p, i) => participantIndex.set(p.id, i));

    const totalWidth =
      participants.length * (PARTICIPANT_WIDTH + PARTICIPANT_GAP) -
      PARTICIPANT_GAP +
      40;

    const { totalHeight, layouts } = measureEvents(events, LIFELINE_START);
    const svgHeight =
      LIFELINE_START + totalHeight + BOTTOM_MARGIN + PARTICIPANT_HEIGHT + 10;

    const lifelineEndY = svgHeight - BOTTOM_MARGIN - PARTICIPANT_HEIGHT;

    return {
      colors,
      participantIndex,
      totalWidth,
      svgHeight,
      lifelineEndY,
      layouts
    };
  }, [diagram, resolvedTheme]);

  if (!mounted) return null;

  const {
    colors,
    participantIndex,
    totalWidth,
    svgHeight,
    lifelineEndY,
    layouts
  } = rendered;
  const { participants, events } = diagram;

  return (
    <div
      className="my-6 rounded-lg border overflow-x-auto"
      style={{ backgroundColor: colors.bg }}
    >
      <svg
        width={Math.max(totalWidth, 300)}
        height={svgHeight}
        viewBox={`-20 0 ${Math.max(totalWidth, 300) + 40} ${svgHeight}`}
        style={{ display: "block", margin: "0 auto" }}
      >
        {/* Lifelines */}
        {participants.map((p, i) => {
          const x = participantX(i);
          return (
            <line
              key={`lifeline-${p.id}`}
              x1={x}
              y1={TOP_MARGIN + PARTICIPANT_HEIGHT}
              x2={x}
              y2={lifelineEndY}
              stroke={colors.lifeline}
              strokeWidth={1}
              strokeDasharray="4 3"
            />
          );
        })}

        {/* Top participant boxes */}
        {participants.map((p, i) => {
          const x = participantX(i);
          const isActor = p.type === "actor";
          return (
            <g key={`top-${p.id}`}>
              <rect
                x={x - PARTICIPANT_WIDTH / 2}
                y={TOP_MARGIN}
                width={PARTICIPANT_WIDTH}
                height={PARTICIPANT_HEIGHT}
                fill={colors.bg}
                stroke={colors.border}
                strokeWidth={1.5}
                rx={4}
              />
              <text
                x={x}
                y={TOP_MARGIN + PARTICIPANT_HEIGHT / 2 + 4}
                textAnchor="middle"
                fill={colors.text}
                fontSize={12}
                fontWeight={500}
                fontFamily="var(--font-mono, monospace)"
              >
                {isActor ? `🧑 ${p.label}` : p.label}
              </text>
            </g>
          );
        })}

        {/* Bottom participant boxes */}
        {participants.map((p, i) => {
          const x = participantX(i);
          return (
            <g key={`bottom-${p.id}`}>
              <rect
                x={x - PARTICIPANT_WIDTH / 2}
                y={lifelineEndY}
                width={PARTICIPANT_WIDTH}
                height={PARTICIPANT_HEIGHT}
                fill={colors.bg}
                stroke={colors.border}
                strokeWidth={1.5}
                rx={4}
              />
              <text
                x={x}
                y={lifelineEndY + PARTICIPANT_HEIGHT / 2 + 4}
                textAnchor="middle"
                fill={colors.text}
                fontSize={12}
                fontWeight={500}
                fontFamily="var(--font-mono, monospace)"
              >
                {p.label}
              </text>
            </g>
          );
        })}

        {/* Events */}
        {renderEvents(events, layouts, participantIndex, colors, "root")}
      </svg>
    </div>
  );
}
