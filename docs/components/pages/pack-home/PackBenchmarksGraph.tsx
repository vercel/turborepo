import cn from "classnames";
import {
  animate,
  motion,
  useInView,
  useAnimation,
  AnimationPlaybackControls,
} from "framer-motion";
import { useTheme } from "next-themes";
import Image from "next/future/image";
import { useEffect, useRef, useState } from "react";
import benchmarkData from "./benchmark-data.json";
import { FadeIn } from "./FadeIn";
import { Gradient } from "./Gradient";
import gradients from "./gradients.module.css";
import { BenchmarkCategory, BenchmarkNumberOfModules } from "./PackBenchmarks";

export function BenchmarksGraph({
  category,
  numberOfModules,
}: {
  category: BenchmarkCategory;
  numberOfModules: BenchmarkNumberOfModules;
}) {
  const data = benchmarkData[category][numberOfModules];
  const keys = Object.keys(data);
  const longestTime = Math.max(...keys.map((key) => data[key])) * 1000;
  const roundedLongestTime = Math.ceil(longestTime / 5000) * 5000 + 5000;
  const graphRef = useRef(null);
  const graphInView = useInView(graphRef, { once: true });

  return (
    <div className="flex w-full max-w-[1280px] relative px-6">
      <div className="flex items-center justify-center flex-1 absolute top-0  w-full h-full">
        <Gradient
          conic
          width="100%"
          height="100%"
          className="dark:opacity-20 dark:md:opacity-25 opacity-10"
        />
      </div>
      <div className="w-40 hidden md:flex flex-col gap-10 ">
        <GraphLabel label="Next.js 13" turbo />
        <GraphLabel label="Next.js 12" />
        <GraphLabel label="Vite" esbuild />
        <GraphLabel label="Next.js 11" />
      </div>
      <div
        ref={graphRef}
        className="relative flex flex-col flex-1 md:gap-10 gap-6"
      >
        <div className="absolute hidden md:flex w-full h-full py-12 -mt-12 box-content">
          <GraphLines />
        </div>
        <div>
          <GraphLabel mobileOnly label="Next.js 13" turbo />
          <GraphBar
            turbo
            duration={data.next13 * 1000}
            longestTime={roundedLongestTime}
            inView={graphInView}
          />
        </div>
        <div>
          <GraphLabel mobileOnly label="Next.js 12" />
          <GraphBar
            duration={data.next12 * 1000}
            longestTime={roundedLongestTime}
            inView={graphInView}
          />
        </div>
        <div>
          <GraphLabel mobileOnly label="Vite" esbuild />
          <GraphBar
            duration={data.vite * 1000}
            longestTime={roundedLongestTime}
            inView={graphInView}
          />
        </div>
        <div>
          <GraphLabel mobileOnly label="Next.js 11" />
          <GraphBar
            duration={data.next11 * 1000}
            longestTime={roundedLongestTime}
            inView={graphInView}
          />
        </div>
      </div>
    </div>
  );
}

const START_DELAY = 0.0;

const graphBarVariants = {
  initial: {
    width: 0,
  },
  progress: {
    width: "100%",
  },
};

const graphBarWrapperVariants = {
  hidden: {
    opacity: 0,
  },
  show: {
    opacity: 1,
  },
};

function GraphBar({
  turbo,
  duration,
  longestTime,
  inView,
}: {
  turbo?: boolean;
  duration: number;
  longestTime: number;
  inView?: boolean;
}) {
  const controls = useAnimation();
  const [timer, setTimer] = useState(0);
  const [timerAnimation, setTimerAnimation] =
    useState<AnimationPlaybackControls>();
  const [barWidth, setBarWidth] = useState(0);
  const [finished, setFinished] = useState(false);

  async function stopAnimation() {
    timerAnimation && timerAnimation.stop();
    controls.stop();
  }

  async function resetAnimation() {
    setTimer(0);
    setFinished(false);
    await controls.start("initial");
  }

  async function startAnimation() {
    const transition = {
      duration: duration / 1000,
      delay: START_DELAY,
    };
    setBarWidth((duration / longestTime) * 100);
    await controls.start("show");
    controls
      .start("progress", {
        ...transition,
        ease: "linear",
      })
      .then(() => {
        setFinished(true);
      });
    const timerAnimationRef = animate(0, duration / 1000, {
      ...transition,
      ease: "linear",
      onUpdate(value) {
        setTimer(value);
      },
    });
    setTimerAnimation(timerAnimationRef);
  }

  async function playFullAnimation() {
    await stopAnimation();
    await controls.start("hidden");
    await resetAnimation();
    await startAnimation();
  }

  useEffect(() => {
    if (inView) {
      void startAnimation();
    } else {
      void stopAnimation();
      void resetAnimation();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inView]);

  useEffect(() => {
    if (!inView) return;
    void playFullAnimation();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [duration, longestTime]);

  return (
    <motion.div
      animate={{ opacity: finished ? 1 : 0.7 }}
      className="flex items-center justify-between gap-4 z-10 dark:bg-[#ffffff0c] bg-[#0000000c] md:bg-transparent dark:md:bg-transparent rounded-lg p-1"
    >
      <motion.div
        animate={controls}
        variants={graphBarWrapperVariants}
        style={{ width: `${barWidth}%` }}
        transition={{ duration: 0.2 }}
        className="flex items-center h-full rounded-lg md:rounded-l-none relative dark:bg-[#ffffff0c] bg-[#0000000c]"
      >
        <motion.div
          className={cn(
            "h-12 rounded-r-lg rounded-l-lg md:rounded-l-none w-0 relative",
            turbo ? gradients.benchmarkTurbo : gradients.benchmark
          )}
          variants={graphBarVariants}
          animate={controls}
          transition={{ duration: 0.2 }}
        >
          <div className="absolute -right-8 w-4 h-12 flex items-center">
            <GraphTimer turbo={turbo} timer={timer} />
          </div>
        </motion.div>
      </motion.div>
      <motion.div className="pr-2">
        <GraphTimer turbo={turbo} timer={timer} mobileOnly />
      </motion.div>
    </motion.div>
  );
}

const GraphTimer = ({
  turbo,
  mobileOnly,
  timer,
}: {
  turbo: boolean;
  timer: number;
  mobileOnly?: boolean;
}) => {
  return (
    <div
      className={`flex flex-row gap-2 items-center z-10 ${
        mobileOnly ? "md:hidden" : "hidden md:flex"
      }`}
    >
      {turbo && (
        <div className="w-8 h-8 flex ">
          <Image
            alt="Turbopack"
            src="/images/docs/pack/turbo-benchmark-icon-light.svg"
            width={32}
            height={32}
            className="block dark:hidden"
          />
          <Image
            alt="Turbopack"
            src="/images/docs/pack/turbo-benchmark-icon-dark.svg"
            width={32}
            height={32}
            className="hidden dark:block"
          />
        </div>
      )}
      <p className="font-mono">{timer.toFixed(1)}s</p>
    </div>
  );
};

function GraphLabel({
  label,
  turbo,
  mobileOnly,
  esbuild,
}: {
  label: string;
  turbo?: boolean;
  mobileOnly?: boolean;
  esbuild?: boolean;
}) {
  return (
    <div
      className={`flex items-center h-12 whitespace-nowrap font-bold gap-y-1 gap-x-2 ${
        mobileOnly && "md:hidden"
      }`}
    >
      <p>{label}</p>
      {turbo && (
        <p className={cn("font-mono m-0", gradients.benchmarkTurboLabel)}>
          turbo
        </p>
      )}
      {esbuild && <p className="font-mono m-0 text-[#666666]">esbuild</p>}
    </div>
  );
}

function GraphLines() {
  const { resolvedTheme } = useTheme();
  const [lineColor, setLineColor] = useState("transparent");

  useEffect(() => {
    setLineColor(resolvedTheme === "dark" ? "white" : "black");
  }, [resolvedTheme]);

  const majorLines = 4;
  const minorLines = 5;
  return (
    <div className="absolute flex flex-1 w-full top-0 bottom-0 opacity-50 z-10">
      <div
        className={cn(
          "w-[1px] h-full absolute left-1 z-20",
          gradients.benchmarkGraphLine
        )}
      />
      {Array.from({ length: majorLines }).map((_, i) => (
        <div className="relative w-full" key={`grid-minor-ticks-${i}`}>
          {Array.from({ length: minorLines }).map((_, i) =>
            i > 0 ? (
              <svg
                key={`grid-minor-tick-${i}`}
                className={`w-[1px] h-full absolute`}
                xmlns="http://www.w3.org/2000/svg"
                style={{ left: `${(i / minorLines) * 100}%` }}
              >
                <defs>
                  <linearGradient
                    id="dashed_line_gradient"
                    x1="0"
                    y1="0"
                    x2="0"
                    y2="100%"
                    gradientUnits="userSpaceOnUse"
                  >
                    <stop offset="0%" stopColor={lineColor} stopOpacity={0} />
                    <stop
                      offset="30%"
                      stopColor={lineColor}
                      stopOpacity={0.3}
                    />
                    <stop
                      offset="70%"
                      stopColor={lineColor}
                      stopOpacity={0.3}
                    />
                    <stop offset="100%" stopColor={lineColor} stopOpacity={0} />
                  </linearGradient>
                </defs>
                <line
                  strokeDasharray="5, 10"
                  x1="0"
                  y1="600"
                  x2="1"
                  y2="1"
                  stroke="url(#dashed_line_gradient)"
                  strokeWidth={5}
                />
              </svg>
            ) : null
          )}
        </div>
      ))}
      {Array.from({ length: majorLines }).map((_, i) => (
        <div
          key={`grid-${i}`}
          className={cn(
            `w-[1px] h-full absolute ml-[-1px] mix-blend-overlay`,
            gradients.benchmarkGraphLine
          )}
          style={{ left: `${((i + 1) / majorLines) * 100}%` }}
        />
      ))}
    </div>
  );
}
