"use client";

import { useEffect, useRef, useState } from "react";

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
const GAUGE_INDICATOR_REST = 80.952;

const roundSvgValue = (value: number) => Math.round(value * 100) / 100;
const GAUGE_GAP_STROKE_WIDTH = roundSvgValue(GAUGE_GAP * 7.9);

function useSpring(target: number) {
  const [value, setValue] = useState(target);
  const valueRef = useRef(target);
  const velocityRef = useRef(0);

  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      valueRef.current = target;
      velocityRef.current = 0;
      setValue(target);
      return;
    }

    let animationFrame: number;
    let previousTime: number | undefined;

    const update = (time: number) => {
      const delta = previousTime
        ? Math.min((time - previousTime) / 1000, 1 / 30)
        : 1 / 60;
      const displacement = target - valueRef.current;
      const acceleration = displacement * 520 - velocityRef.current * 22;

      velocityRef.current += acceleration * delta;
      valueRef.current += velocityRef.current * delta;
      previousTime = time;

      if (
        Math.abs(target - valueRef.current) < 0.01 &&
        Math.abs(velocityRef.current) < 0.01
      ) {
        valueRef.current = target;
        velocityRef.current = 0;
        setValue(target);
        return;
      }

      setValue(valueRef.current);
      animationFrame = requestAnimationFrame(update);
    };

    animationFrame = requestAnimationFrame(update);

    return () => cancelAnimationFrame(animationFrame);
  }, [target]);

  return value;
}

const GAUGE_GAP_LINES = GAUGE_SEGMENTS.slice(0, -1).map(({ length, start }) => {
  const progress = (start + length) / 100;
  const angle = ((160 + progress * 220) * Math.PI) / 180;

  return {
    x1: roundSvgValue(240 + Math.cos(angle) * 140),
    x2: roundSvgValue(240 + Math.cos(angle) * 270),
    y1: roundSvgValue(240 + Math.sin(angle) * 140),
    y2: roundSvgValue(240 + Math.sin(angle) * 270),
  };
});

const GAUGE_TICKS = GAUGE_SEGMENTS.flatMap(({ length, start }, segmentIndex) =>
  Array.from(
    { length: segmentIndex === GAUGE_SEGMENTS.length - 1 ? 7 : 6 },
    (_, index) => {
      const progress = (start + (length * (index + 1)) / 7) / 100;
      const angle = ((160 + progress * 220) * Math.PI) / 180;

      return {
        x1: roundSvgValue(240 + Math.cos(angle) * 188),
        x2: roundSvgValue(240 + Math.cos(angle) * 196),
        y1: roundSvgValue(240 + Math.sin(angle) * 188),
        y2: roundSvgValue(240 + Math.sin(angle) * 196),
      };
    },
  ),
);

export function RemoteCacheGauge() {
  const [isAccelerating, setIsAccelerating] = useState(false);
  const [springTarget, setSpringTarget] = useState(GAUGE_INDICATOR_REST);
  const indicatorProgress = useSpring(springTarget);
  const roundedIndicatorProgress = roundSvgValue(
    Math.min(100, Math.max(0, indicatorProgress)),
  );
  const indicatorRemainder = roundSvgValue(100 - roundedIndicatorProgress);
  const indicatorDasharray = `${roundedIndicatorProgress} ${indicatorRemainder}`;
  const indicatorRemainderDasharray = `0 ${roundedIndicatorProgress} ${indicatorRemainder} 0`;
  const needleRotation = roundSvgValue(roundedIndicatorProgress * 2.2);

  useEffect(() => {
    if (!isAccelerating) {
      setSpringTarget(GAUGE_INDICATOR_REST);
      return;
    }

    let flickerTimeout: ReturnType<typeof setTimeout>;
    let flickerStep = 0;
    const flickerDurations = [70, 95, 80, 110] as const;

    setSpringTarget(98);

    const flicker = (towardLimit: boolean) => {
      setSpringTarget(towardLimit ? 97.8 : 96.2);
      const duration = flickerDurations[flickerStep % flickerDurations.length];
      flickerStep += 1;
      flickerTimeout = setTimeout(() => flicker(!towardLimit), duration);
    };

    flickerTimeout = setTimeout(() => flicker(false), 280);

    return () => {
      clearTimeout(flickerTimeout);
    };
  }, [isAccelerating]);

  return (
    <div
      className="relative w-full max-w-[500px] aspect-[24/17] [--remote-cache-inner-end:#FF1E56] [--remote-cache-inner-start:#0196FF] [--remote-cache-tick-color:var(--ds-gray-alpha-500)] dark:[--remote-cache-inner-end:#FF5C9A] dark:[--remote-cache-inner-start:#52C7FF] dark:[--remote-cache-tick-color:var(--ds-gray-alpha-600)]"
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
            <stop offset="0" stopColor="#0196FF" />
            <stop offset="1" stopColor="#FF1E56" />
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
          <linearGradient
            id="remote-cache-gauge-pointer-gradient"
            gradientUnits="userSpaceOnUse"
            x1="236.58"
            x2="243.42"
            y1="230.6"
            y2="249.4"
          >
            <stop offset="0" stopColor="var(--ds-gray-700)" />
            <stop offset="0.42" stopColor="var(--ds-gray-1000)" />
            <stop offset="1" stopColor="var(--ds-gray-800)" />
          </linearGradient>
          <radialGradient
            id="remote-cache-gauge-dot-gradient"
            gradientUnits="userSpaceOnUse"
            cx="234"
            cy="233"
            fx="234"
            fy="233"
            r="27"
          >
            <stop offset="0" stopColor="var(--ds-gray-700)" />
            <stop offset="0.48" stopColor="var(--ds-gray-1000)" />
            <stop offset="1" stopColor="var(--ds-gray-700)" />
          </radialGradient>
          <radialGradient
            id="remote-cache-gauge-pin-gradient"
            cx="35%"
            cy="30%"
          >
            <stop offset="0" stopColor="var(--ds-gray-100)" />
            <stop offset="1" stopColor="var(--ds-gray-400)" />
          </radialGradient>
          <radialGradient
            id="remote-cache-gauge-fade"
            cx="240"
            cy="240"
            gradientUnits="userSpaceOnUse"
            r="197"
          >
            <stop offset="75%" stopColor="white" stopOpacity="0" />
            <stop offset="81%" stopColor="white" stopOpacity="0.03" />
            <stop offset="87%" stopColor="white" stopOpacity="0.12" />
            <stop offset="92%" stopColor="white" stopOpacity="0.32" />
            <stop offset="95%" stopColor="white" stopOpacity="0.64" />
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
            <g stroke="black" strokeWidth={GAUGE_GAP_STROKE_WIDTH}>
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
          <filter
            id="remote-cache-gauge-pointer-shadow-blur"
            height="140%"
            width="140%"
            x="-20%"
            y="-20%"
          >
            <feGaussianBlur stdDeviation="1.2" />
          </filter>
        </defs>

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
        </g>

        <g mask="url(#remote-cache-gauge-segment-mask)">
          <g mask="url(#remote-cache-gauge-end-fade-mask)">
            <path
              d="M46.4 310.5a206 206 0 1 1 387.2 0"
              fill="none"
              stroke="var(--ds-gray-alpha-300)"
              strokeLinecap="butt"
              strokeWidth="18"
            />
            <g mask="url(#remote-cache-gauge-inner-indicator-mask)">
              <path
                className="opacity-30 dark:opacity-25"
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

        <g
          className="mix-blend-multiply dark:mix-blend-normal"
          stroke="var(--remote-cache-tick-color)"
          strokeWidth="1"
        >
          {GAUGE_TICKS.map(({ x1, x2, y1, y2 }, index) => (
            <line key={index} x1={x1} x2={x2} y1={y1} y2={y2} />
          ))}
        </g>

        <g transform="translate(1 2)">
          <g transform={`rotate(${needleRotation} 240 240)`}>
            <path
              d="M236.58 230.6 103.74 289.59 243.42 249.4Z"
              fill="black"
              fillOpacity="0.22"
              filter="url(#remote-cache-gauge-pointer-shadow-blur)"
            />
          </g>
        </g>

        <g transform={`rotate(${needleRotation} 240 240)`}>
          <path
            d="M236.58 230.6 103.74 289.59 243.42 249.4Z"
            fill="url(#remote-cache-gauge-pointer-gradient)"
          />
          <path
            d="M236.58 230.6 103.74 289.59"
            fill="none"
            stroke="white"
            strokeOpacity="0.16"
            strokeWidth="0.75"
          />
        </g>

        <g>
          <circle cx="240" cy="240" fill="var(--ds-gray-alpha-500)" r="20" />
          <circle
            cx="240"
            cy="240"
            fill="url(#remote-cache-gauge-dot-gradient)"
            r="18"
          />
          <ellipse
            cx="235"
            cy="233.5"
            fill="white"
            fillOpacity="0.16"
            rx="7"
            ry="3"
            transform="rotate(-25 235 233.5)"
          />
          <circle
            cx="240"
            cy="240"
            fill="url(#remote-cache-gauge-pin-gradient)"
            r="7"
            stroke="var(--ds-gray-alpha-500)"
            strokeWidth="1"
          />
        </g>
      </svg>
    </div>
  );
}
