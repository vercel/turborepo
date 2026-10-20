import { IconCheck } from "@vercel/geistdocs/assets/icons/icon-check";
import type { ReactNode } from "react";

const validationTasks = [
  { name: "lint", time: "320ms" },
  { name: "test", time: "680ms" },
  { name: "build", time: "1.1s" },
];

function ChatTail({ side }: { side: "left" | "right" }) {
  return (
    <svg
      aria-hidden="true"
      className={`absolute -bottom-[3px] ${
        side === "left" ? "-left-[7px]" : "-right-[7px] scale-x-[-1]"
      }`}
      fill="none"
      height="24"
      viewBox="0 0 24 24"
      width="24"
    >
      <path d="M0 0h24v24H0z" fill="var(--ds-background-200)" />
      {/* x/y are required: a mask defaults to x="-10%" y="-10%", which would
          shift the region off the tile and clip the curl's outline and shadow. */}
      <mask
        height="24"
        id={`chat-tail-mask-${side}`}
        maskUnits="userSpaceOnUse"
        style={{ maskType: "alpha" }}
        width="24"
        x="0"
        y="0"
      >
        <path d="M0 0h24v24H0z" fill="white" />
      </mask>
      <g
        filter={`url(#chat-tail-shadow-${side})`}
        mask={`url(#chat-tail-mask-${side})`}
      >
        <mask
          fill="black"
          height="42"
          id={`chat-tail-outline-${side}`}
          maskUnits="userSpaceOnUse"
          width="46"
          x="2"
          y="-20"
        >
          <path d="M2-20h46v42H2z" fill="white" />
          <path
            clipRule="evenodd"
            d="M27-19C15.954-19 7-10.046 7 1c0 .335.008.669.025 1H7v10a15 15 0 0 1-3 9c4.116 0 7.845-1.658 10.555-4.342A19.915 19.915 0 0 0 27 21c11.046 0 20-8.954 20-20s-8.954-20-20-20Z"
            fillRule="evenodd"
          />
        </mask>
        <path
          clipRule="evenodd"
          d="M27-19C15.954-19 7-10.046 7 1c0 .335.008.669.025 1H7v10a15 15 0 0 1-3 9c4.116 0 7.845-1.658 10.555-4.342A19.915 19.915 0 0 0 27 21c11.046 0 20-8.954 20-20s-8.954-20-20-20Z"
          fill="var(--ds-background-100)"
          fillRule="evenodd"
        />
        <path
          d="M7.025 2v1h1.05l-.052-1.05-.998.05ZM7 2V1H6v1h1ZM4 21l-.8-.6L2 22h2v-1Zm10.555-4.342.623-.783-.695-.553-.631.625.703.71ZM8 1c0-10.493 8.507-19 19-19v-2C15.402-20 6-10.598 6 1h2Zm.023.95A19.327 19.327 0 0 1 8 1H6c0 .352.009.702.026 1.05l1.997-.1ZM7 3h.025V1H7v2Zm1 9V2H6v10h2Zm-3.2 9.6A16 16 0 0 0 8 12H6a14 14 0 0 1-2.8 8.4l1.6 1.2Zm9.052-5.653A13.952 13.952 0 0 1 4 20v2c4.39 0 8.37-1.77 11.259-4.632l-1.407-1.42ZM27 20c-4.47 0-8.577-1.542-11.822-4.125l-1.245 1.565A20.915 20.915 0 0 0 27 22v-2ZM46 1c0 10.493-8.507 19-19 19v2c11.598 0 21-9.402 21-21h-2ZM27-18c10.493 0 19 8.507 19 19h2c0-11.598-9.402-21-21-21v2Z"
          fill="var(--ds-gray-alpha-400)"
          mask={`url(#chat-tail-outline-${side})`}
        />
      </g>
      <defs>
        <filter
          colorInterpolationFilters="sRGB"
          filterUnits="userSpaceOnUse"
          height="46"
          id={`chat-tail-shadow-${side}`}
          width="50"
          x="0"
          y="-21"
        >
          {/* Matches --ds-shadow-small. Kept literally black rather than a gray
              token, which would invert to a white glow in dark mode. */}
          <feDropShadow
            dx="0"
            dy="1"
            floodColor="#000"
            floodOpacity="0.04"
            stdDeviation="1"
          />
        </filter>
      </defs>
    </svg>
  );
}

function ChatBubble({
  align,
  children,
}: {
  align: "start" | "end";
  children: ReactNode;
}) {
  return (
    <div
      className={`flex ${align === "end" ? "justify-end" : "justify-start"}`}
    >
      <div className="relative max-w-[88%] rounded-[20px] bg-background-100 px-4 py-3 shadow-[var(--ds-shadow-border-small)]">
        {children}
        <ChatTail side={align === "end" ? "right" : "left"} />
      </div>
    </div>
  );
}

export function ValidationLoopsVisual() {
  return (
    <div
      aria-hidden="true"
      className="flex size-full items-center justify-center bg-background-200 px-6"
    >
      <div className="w-full max-w-[280px]">
        <div className="relative flex w-full flex-col">
          <code className="w-full border-green-900 border-l-2 bg-green-300 px-3 py-2 text-green-1000 text-label-14-mono">
            + app/dashboard
          </code>
          <div
            aria-hidden="true"
            className="pointer-events-none absolute -top-px -right-px -bottom-px w-1/2 bg-linear-to-r from-transparent to-background-200"
          />
        </div>
        <div className="mt-5 flex flex-col gap-2.5 text-label-13-mono">
          {validationTasks.map((task) => (
            <div className="flex items-center gap-2" key={task.name}>
              <IconCheck className="shrink-0 text-green-900" size={14} />
              <span className="flex-1 text-gray-900">{task.name}</span>
              <span className="text-gray-600">{task.time}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

const skills = ["turborepo", "task graphs", "workspaces"];

export function AgentSkillsVisual() {
  return (
    <div
      aria-hidden="true"
      className="relative flex size-full flex-col justify-center bg-background-200 px-6 py-5"
    >
      <div className="mx-auto flex w-[85%] flex-col gap-3">
        <ChatBubble align="end">
          <p className="text-copy-13 text-gray-1000">
            Help me update this monorepo
          </p>
        </ChatBubble>

        <ChatBubble align="start">
          <div className="flex flex-col gap-2 pr-6">
            <p className="text-copy-13 text-gray-1000">Loading skills...</p>
            <ul className="flex flex-col gap-1">
              {skills.map((skill) => (
                <li
                  className="flex items-center gap-2 text-gray-1000 text-label-13-mono"
                  key={skill}
                >
                  <IconCheck className="shrink-0 text-green-900" size={14} />
                  {skill}
                </li>
              ))}
            </ul>
            <p className="text-copy-13 text-gray-1000">3 skills loaded</p>
          </div>
        </ChatBubble>
      </div>

      <div className="pointer-events-none absolute inset-x-0 top-0 h-14 bg-linear-to-b from-background-200 to-transparent" />
      <div className="pointer-events-none absolute inset-x-0 bottom-0 h-14 bg-linear-to-t from-background-200 to-transparent" />
    </div>
  );
}

export function TurboDocsVisual() {
  return (
    <div
      aria-hidden="true"
      className="flex size-full items-center justify-center bg-background-200 px-6"
    >
      <div className="relative w-full max-w-[310px]">
        <div className="overflow-hidden rounded-lg bg-background-200 shadow-(--ds-shadow-border-small)">
          <div className="flex h-10 items-center border-gray-300 border-b px-3">
            <div className="flex gap-1.5">
              <span className="size-2 rounded-full bg-[#EE6D5E]" />
              <span className="size-2 rounded-full bg-[#F3BF4A]" />
              <span className="size-2 rounded-full bg-[#5DC753]" />
            </div>
          </div>
          <div className="h-[150px] px-4 py-4 text-label-13-mono">
            <p className="text-gray-1000">
              <span className="text-gray-600">$</span> turbo docs "remote caching"
            </p>
            <div className="mt-4 flex flex-col gap-1.5">
              <p className="text-gray-600">Searching Turborepo docs...</p>
              <p className="text-gray-1000">Remote Caching</p>
              <p className="text-gray-600">
                Share cached task outputs across machines.
              </p>
            </div>
          </div>
        </div>
        <div className="pointer-events-none absolute -inset-x-1 -bottom-1 h-24 bg-linear-to-t from-background-200 to-transparent" />
      </div>
    </div>
  );
}
