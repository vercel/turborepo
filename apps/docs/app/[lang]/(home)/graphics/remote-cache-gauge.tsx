"use client";

import { easeOutCubic } from "phase/ease";
import { useTween } from "phase/react";
import { useEffect, useState } from "react";

import { RemoteCacheCounterClient } from "@/components/remote-cache-counter/client";

const GAUGE_SEGMENTS = [
  { length: 14.285714, start: 0 },
  { length: 14.285714, start: 14.285714 },
  { length: 14.285714, start: 28.571428 },
  { length: 14.285714, start: 42.857142 },
  { length: 14.285714, start: 57.142856 },
  { length: 14.285714, start: 71.42857 },
  { length: 14.285716, start: 85.714284 },
] as const;

const GAUGE_GAP = 0.35;
const GAUGE_INDICATOR_START = 61.905;
const GAUGE_INDICATOR_REST = 78.571;

const roundSvgCoordinate = (value: number) => Math.round(value * 1000) / 1000;

const GAUGE_GAP_LINES = GAUGE_SEGMENTS.slice(0, -1).map(
  ({ length, start }) => {
    const progress = (start + length) / 100;
    const angle = ((160 + progress * 220) * Math.PI) / 180;

    return {
      x1: roundSvgCoordinate(240 + Math.cos(angle) * 140),
      x2: roundSvgCoordinate(240 + Math.cos(angle) * 270),
      y1: roundSvgCoordinate(240 + Math.sin(angle) * 140),
      y2: roundSvgCoordinate(240 + Math.sin(angle) * 270),
    };
  },
);

const GAUGE_TICKS = GAUGE_SEGMENTS.flatMap(({ length, start }, segmentIndex) =>
  Array.from(
    { length: segmentIndex === GAUGE_SEGMENTS.length - 1 ? 7 : 6 },
    (_, index) => {
      const progress = (start + (length * (index + 1)) / 7) / 100;
      const angle = ((160 + progress * 220) * Math.PI) / 180;

      return {
        x1: roundSvgCoordinate(240 + Math.cos(angle) * 188),
        x2: roundSvgCoordinate(240 + Math.cos(angle) * 196),
        y1: roundSvgCoordinate(240 + Math.sin(angle) * 188),
        y2: roundSvgCoordinate(240 + Math.sin(angle) * 196),
      };
    },
  ),
);

export function RemoteCacheGauge() {
  const [isAccelerating, setIsAccelerating] = useState(false);
  const [hasStarted, setHasStarted] = useState(false);
  const indicatorProgress = useTween({
    duration: 1200,
    easing: easeOutCubic,
    target: isAccelerating
      ? 100
      : hasStarted
        ? GAUGE_INDICATOR_REST
        : GAUGE_INDICATOR_START,
  });
  const indicatorDasharray = `${indicatorProgress} ${100 - indicatorProgress}`;
  const indicatorRemainderDasharray = `0 ${indicatorProgress} ${100 - indicatorProgress} 0`;

  useEffect(() => setHasStarted(true), []);

  return (
    <div
      className="relative w-full max-w-[500px] aspect-[24/17] [--remote-cache-inner-end:#0196FF] [--remote-cache-inner-start:#FF1E56] dark:[--remote-cache-inner-end:#52C7FF] dark:[--remote-cache-inner-start:#FF5C9A]"
      onMouseEnter={() => setIsAccelerating(true)}
      onMouseLeave={() => setIsAccelerating(false)}
    >
      <svg
        aria-hidden="true"
        className="absolute inset-0 size-full overflow-visible"
        viewBox="0 0 480 340"
      >
        <defs>
          <linearGradient
            id="remote-cache-gauge-gradient"
            gradientUnits="userSpaceOnUse"
            x1="46.4"
            x2="433.6"
            y1="310.5"
            y2="310.5"
          >
            <stop offset="0" stopColor="#FF1E56" />
            <stop offset="1" stopColor="#0196FF" />
          </linearGradient>
          <linearGradient
            id="remote-cache-gauge-inner-gradient"
            gradientUnits="userSpaceOnUse"
            x1="46.4"
            x2="433.6"
            y1="310.5"
            y2="310.5"
          >
            <stop offset="0" stopColor="var(--remote-cache-inner-start)" />
            <stop offset="1" stopColor="var(--remote-cache-inner-end)" />
          </linearGradient>
          <radialGradient
            id="remote-cache-gauge-fade"
            cx="240"
            cy="240"
            gradientUnits="userSpaceOnUse"
            r="197"
          >
            <stop offset="80%" stopColor="white" stopOpacity="0" />
            <stop offset="100%" stopColor="white" stopOpacity="1" />
          </radialGradient>
          <mask id="remote-cache-gauge-fade-mask" mask-type="alpha">
            <rect
              fill="url(#remote-cache-gauge-fade)"
              height="340"
              width="480"
            />
          </mask>
          <mask id="remote-cache-gauge-segment-mask" mask-type="luminance">
            <path
              d="M46.4 310.5a206 206 0 1 1 387.2 0"
              fill="none"
              stroke="white"
              strokeLinecap="butt"
              strokeWidth="120"
            />
            <g stroke="black" strokeWidth={GAUGE_GAP * 7.9}>
              {GAUGE_GAP_LINES.map(({ x1, x2, y1, y2 }, index) => (
                <line key={index} x1={x1} x2={x2} y1={y1} y2={y2} />
              ))}
            </g>
          </mask>
          <mask id="remote-cache-gauge-inner-indicator-mask" mask-type="alpha">
            <path
              d="M54.9 307.4a197 197 0 1 1 370.2 0"
              fill="none"
              pathLength="100"
              stroke="white"
              strokeDasharray={indicatorDasharray}
              strokeLinecap="butt"
              strokeWidth="394"
            />
          </mask>
          <mask id="remote-cache-gauge-inner-remainder-mask" mask-type="alpha">
            <path
              d="M54.9 307.4a197 197 0 1 1 370.2 0"
              fill="none"
              pathLength="100"
              stroke="white"
              strokeDasharray={indicatorRemainderDasharray}
              strokeLinecap="butt"
              strokeWidth="394"
            />
          </mask>
          <linearGradient
            id="remote-cache-gauge-end-fade"
            gradientUnits="userSpaceOnUse"
            x1="0"
            x2="0"
            y1="310.5"
            y2="270"
          >
            <stop offset="0" stopColor="white" stopOpacity="0" />
            <stop offset="100%" stopColor="white" />
          </linearGradient>
          <mask id="remote-cache-gauge-end-fade-mask" mask-type="alpha">
            <rect
              fill="url(#remote-cache-gauge-end-fade)"
              height="340"
              width="480"
            />
          </mask>
          <filter
            id="remote-cache-gauge-glow"
            height="160%"
            width="160%"
            x="-30%"
            y="-30%"
          >
            <feGaussianBlur stdDeviation="2.5" />
          </filter>
        </defs>

        <g mask="url(#remote-cache-gauge-segment-mask)">
          <g mask="url(#remote-cache-gauge-end-fade-mask)">
            <path
              className="opacity-25 dark:mix-blend-screen dark:opacity-40"
              d="M46.4 310.5a206 206 0 1 1 387.2 0"
              fill="none"
              filter="url(#remote-cache-gauge-glow)"
              pathLength="100"
              stroke="url(#remote-cache-gauge-gradient)"
              strokeDasharray={indicatorDasharray}
              strokeLinecap="butt"
              strokeWidth="21"
            />
            <path
              d="M46.4 310.5a206 206 0 1 1 387.2 0"
              fill="none"
              stroke="var(--ds-gray-alpha-300)"
              strokeLinecap="butt"
              strokeWidth="18"
            />
            <g mask="url(#remote-cache-gauge-inner-indicator-mask)">
              <path
                className="opacity-30 dark:opacity-20"
                d="M54.9 307.4a197 197 0 1 1 370.2 0L240 240Z"
                fill="url(#remote-cache-gauge-inner-gradient)"
                mask="url(#remote-cache-gauge-fade-mask)"
              />
            </g>
            <g mask="url(#remote-cache-gauge-inner-remainder-mask)">
              <path
                className="opacity-30 dark:opacity-20"
                d="M54.9 307.4a197 197 0 1 1 370.2 0L240 240Z"
                fill="var(--ds-gray-alpha-500)"
                mask="url(#remote-cache-gauge-fade-mask)"
              />
            </g>
            <path
              d="M46.4 310.5a206 206 0 1 1 387.2 0"
              fill="none"
              pathLength="100"
              stroke="url(#remote-cache-gauge-gradient)"
              strokeDasharray={indicatorDasharray}
              strokeLinecap="butt"
              strokeWidth="18"
            />
          </g>
        </g>

        <g stroke="var(--ds-gray-alpha-500)" strokeWidth="1">
          {GAUGE_TICKS.map(({ x1, x2, y1, y2 }, index) => (
            <line key={index} x1={x1} x2={x2} y1={y1} y2={y2} />
          ))}
        </g>
      </svg>

      <div className="absolute inset-x-8 top-[50%] flex flex-col items-center text-center">
        <RemoteCacheCounterClient className="min-w-0 bg-gradient-to-r from-[#FF1E56] to-[#0196FF] bg-clip-text pr-1 font-semibold [--font-weight-semibold:450] text-[44px] text-transparent leading-none tracking-[-0.055em] sm:text-[52px] lg:text-[56px]" />
        <p className="mt-2 text-balance max-w-[300px] text-copy-14 text-gray-900">
          compute hours saved with Remote Caching
        </p>
      </div>
    </div>
  );
}
