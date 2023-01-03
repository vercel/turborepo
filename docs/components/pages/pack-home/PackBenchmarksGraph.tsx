import cn from "classnames";
import {
  animate,
  motion,
  useInView,
  useAnimation,
  AnimationPlaybackControls,
} from "framer-motion";
import Image from "next/image";
import { useEffect, useRef, useState } from "react";
import benchmarkData from "./benchmark-data/data.json";
import { Gradient } from "../home-shared/Gradient";
import gradients from "../home-shared/gradients.module.css";
import {
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
          gray
          width="100%"
          height="100%"
          className="dark:opacity-0 dark:md:opacity-25 opacity-10"
        />
      </div>
      <div
        ref={graphRef}
        className="relative flex flex-col flex-1 gap-6 md:gap-10"
      >
        {bars.map((bar) => {
          return (
            <GraphBar
              key={bar.key}
              turbo={bar.turbo}
              Label={
                <GraphLabel label={bar.label} turbo={bar.turbo} swc={bar.swc} />
              }
              duration={data[bar.key] * 1000}
              longestTime={longestTimeWithPadding}
              inView={graphInView}
              pinTime={pinTime}
            ></GraphBar>
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
  const [, setFinished] = useState(false);

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
    <div className="justify-center w-full gap-1 md:flex-row md:flex align-center">
      <div className="flex items-center w-48">{Label}</div>
      <div className="flex w-full items-center justify-between gap-4 z-10 border dark:border-[#333333] rounded-lg p-1">
        <motion.div
          animate={controls}
          variants={graphBarWrapperVariants}
          style={{ width: `${barWidth}%` }}
          transition={{ duration: 0.1 }}
          initial="hidden"
          className={cn(
            "flex items-center h-full rounded relative dark:bg-[#383838] bg-[#E6E6E6]"
          )}
        >
          <motion.div
            className={cn(
              "h-12 rounded w-0 relative",
              turbo ? gradients.benchmarkTurbo : gradients.benchmark,
              { [gradients.barBorder]: !turbo }
            )}
            variants={graphBarVariants}
            animate={controls}
            transition={{ duration: 0.1 }}
          />
        </motion.div>
        <motion.div
          animate={controls}
          variants={graphBarWrapperVariants}
          className="pr-2"
          transition={{ duration: 0.1 }}
        >
          <GraphTimer
            turbo={turbo}
            timer={pinTime ? duration : timer}
            duration={duration}
          />
        </motion.div>
      </div>
    </div>
  );
}

const GraphTimer = ({
  turbo,
  timer,
  duration,
}: {
  turbo: boolean;
  timer: number;
  duration: number;
}) => {
  return (
    <div className={`flex flex-row gap-2 w-24 justify-end items-center z-10`}>
      {turbo && (
        <div className="relative flex w-6 h-6">
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
          <Gradient
            pink
            width="100%"
            height="100%"
            small
            className="opacity-0 dark:opacity-60"
          />
        </div>
      )}
      <p className="font-mono">
        <Time value={timer} maxValue={duration} />
      </p>
    </div>
  );
};

function roundTo(num: number, decimals: number) {
  const factor = Math.pow(10, decimals);
  return Math.round(num * factor) / factor;
}

const Time = ({
  value,
  maxValue,
}: {
  value: number;
  maxValue: number;
}): JSX.Element => {
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
};

function GraphLabel({
  label,
  turbo,
  swc,
  mobileOnly,
  esbuild,
}: {
  label: string;
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
    >
      <p>{label}</p>
      {turbo && (
        <p
          className={cn(
            "font-space-grotesk m-0",
            gradients.benchmarkTurboLabel
          )}
        >
          turbo
        </p>
      )}
      {swc && (
        <p className="font-space-grotesk m-0 font-light text-[#666666]">
          with SWC
        </p>
      )}
      {esbuild && (
        <p className="font-space-grotesk m-0 text-[#666666]">esbuild</p>
      )}
    </div>
  );
}
