import { useEffect, useRef, useState } from "react";
import cn from "classnames";
import type { AnimationPlaybackControls } from "framer-motion";
import { animate, motion, useInView, useAnimation } from "framer-motion";
import Image from "next/image";
import { Gradient } from "../home-shared/Gradient";
import gradients from "../home-shared/gradients.module.css";
import benchmarkData from "./benchmark-data/data.json";
import type {
  BenchmarkBar,
  BenchmarkCategory,
  BenchmarkData,
  BenchmarkNumberOfModules,
} from "./PackBenchmarks";

interface BenchmarksGraphProps {
  category: BenchmarkCategory;
  numberOfModules: BenchmarkNumberOfModules;
  bars: BenchmarkBar[];
  pinTime?: true;
}

export function BenchmarksGraph({
  category,
  numberOfModules,
  bars,
  pinTime,
}: BenchmarksGraphProps) {
  // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment, @typescript-eslint/no-unsafe-member-access -- JSON not typed.
  const data: BenchmarkData = benchmarkData[category][numberOfModules];
  const keys = bars.map((bar) => bar.key);
  const longestTime = Math.max(...keys.map((key) => data[key])) * 1000;
  const longestTimeWithPadding = longestTime * 1.15;
  const graphRef = useRef(null);
  const graphInView = useInView(graphRef, { once: true, margin: "-128px" });

  return (
    <div className="flex w-full max-w-[1248px] relative px-6">
      <div className="absolute top-0 flex items-center justify-center flex-1 w-full h-full">
        <Gradient
          className="dark:opacity-0 dark:md:opacity-25 opacity-10"
          gray
          height="100%"
          width="100%"
        />
      </div>
      <div
        className="relative flex flex-col flex-1 gap-6 md:gap-10"
        ref={graphRef}
      >
        {bars.map((bar) => {
          return (
            <GraphBar
              Label={
                <GraphLabel
                  label={bar.label}
                  swc={bar.swc}
                  turbo={bar.turbo}
                  version={bar.version}
                />
              }
              duration={data[bar.key] * 1000}
              inView={graphInView}
              key={bar.key}
              longestTime={longestTimeWithPadding}
              pinTime={pinTime}
              turbo={bar.turbo}
            />
          );
        })}
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
  Label,
  pinTime,
}: {
  turbo?: boolean;
  duration: number;
  longestTime: number;
  Label: JSX.Element;
  inView?: boolean;
  // Pin the time
  pinTime?: true;
}) {
  const controls = useAnimation();
  const [timer, setTimer] = useState(0);
  const [timerAnimation, setTimerAnimation] =
    useState<AnimationPlaybackControls>();
  const [barWidth, setBarWidth] = useState(0);
  // eslint-disable-next-line react/hook-use-state -- Don't need the value.
  const [, setFinished] = useState(false);

  function stopAnimation() {
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
      })
      .catch(() => {
        setFinished(true);
      });
    const timerAnimationRef = animate(0, duration, {
      ...transition,
      ease: "linear",
      onUpdate(value) {
        setTimer(value);
      },
    });
    setTimerAnimation(timerAnimationRef);
  }

  async function playFullAnimation() {
    stopAnimation();
    await controls.start("hidden");
    await resetAnimation();
    await startAnimation();
  }

  useEffect(() => {
    if (inView) {
      void startAnimation();
    } else {
      stopAnimation();
      void resetAnimation();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- Would make the animation wrong.
  }, [inView]);

  useEffect(() => {
    if (!inView) return;
    void playFullAnimation();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- Would make the animation wrong.
  }, [duration, longestTime]);

  return (
    <div className="justify-center w-full gap-1 md:flex-row md:flex align-center">
      <div className="flex items-center w-48">{Label}</div>
      <div className="flex w-full items-center justify-between gap-4 z-10 border dark:border-[#333333] rounded-lg p-1">
        <motion.div
          animate={controls}
          className={cn(
            "flex items-center h-full rounded relative dark:bg-[#383838] bg-[#E6E6E6]"
          )}
          initial="hidden"
          style={{ width: `${barWidth}%` }}
          transition={{ duration: 0.1 }}
          variants={graphBarWrapperVariants}
        >
          <motion.div
            animate={controls}
            className={cn(
              "h-12 rounded w-0 relative",
              turbo ? gradients.benchmarkTurbo : gradients.benchmark,
              { [gradients.barBorder]: !turbo }
            )}
            transition={{ duration: 0.1 }}
            variants={graphBarVariants}
          />
        </motion.div>
        <motion.div
          animate={controls}
          className="pr-2"
          transition={{ duration: 0.1 }}
          variants={graphBarWrapperVariants}
        >
          <GraphTimer
            duration={duration}
            timer={pinTime ? duration : timer}
            turbo={turbo}
          />
        </motion.div>
      </div>
    </div>
  );
}

function GraphTimer({
  turbo,
  timer,
  duration,
}: {
  turbo?: boolean;
  timer: number;
  duration: number;
}) {
  return (
    <div className="flex flex-row gap-2 w-24 justify-end items-center z-10">
      {turbo ? (
        <div className="relative flex w-6 h-6">
          <Image
            alt="Turbopack"
            className="block dark:hidden"
            height={32}
            src="/images/docs/pack/turbo-benchmark-icon-light.svg"
            width={32}
          />
          <Image
            alt="Turbopack"
            className="hidden dark:block"
            height={32}
            src="/images/docs/pack/turbo-benchmark-icon-dark.svg"
            width={32}
          />
          <Gradient
            className="opacity-0 dark:opacity-60"
            height="100%"
            pink
            small
            width="100%"
          />
        </div>
      ) : null}
      <p className="font-mono">
        <Time maxValue={duration} value={timer} />
      </p>
    </div>
  );
}

function roundTo(num: number, decimals: number) {
  const factor = Math.pow(10, decimals);
  return Math.round(num * factor) / factor;
}

function Time({
  value,
  maxValue,
}: {
  value: number;
  maxValue: number;
}): JSX.Element {
  let unitValue: string;
  let unit: string;
  if (maxValue < 1000) {
    unitValue = Math.round(value).toFixed(0);
    unit = "ms";
  } else {
    const roundedValue = roundTo(value / 1000, 1);
    unitValue = roundedValue.toFixed(1);
    unit = "s";
  }

  return (
    <>
      {unitValue}
      {unit}
    </>
  );
}

function GraphLabel({
  label,
  turbo,
  swc,
  mobileOnly,
  esbuild,
  version,
}: {
  label: string;
  version: string;
  turbo?: boolean;
  swc?: boolean;
  mobileOnly?: boolean;
  esbuild?: boolean;
}) {
  return (
    <div
      className={`flex items-center h-12 whitespace-nowrap font-bold gap-y-1 gap-x-2 ${
        mobileOnly && "md:hidden"
      }`}
      title={version}
    >
      <p>{label}</p>
      {turbo ? (
        <p
          className={cn(
            "font-space-grotesk m-0",
            gradients.benchmarkTurboLabel
          )}
        >
          turbo
        </p>
      ) : null}
      {swc ? (
        <p className="font-space-grotesk m-0 font-light text-[#666666]">
          with SWC
        </p>
      ) : null}
      {esbuild ? (
        <p className="font-space-grotesk m-0 text-[#666666]">esbuild</p>
      ) : null}
    </div>
  );
}
