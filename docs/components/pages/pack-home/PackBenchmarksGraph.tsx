import cn from "classnames";
import {
  animate,
  motion,
  useInView,
  useAnimation,
  AnimationPlaybackControls,
} from "framer-motion";
import Image from "next/future/image";
import { useEffect, useRef, useState } from "react";
import benchmarkData from "./benchmark-data.json";
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
      <div className="flex items-center justify-center flex-1 absolute top-0 w-full h-full">
        <Gradient
          gray
          width="100%"
          height="100%"
          className="dark:opacity-0 dark:md:opacity-25 opacity-10"
        />
      </div>
      <div
        ref={graphRef}
        className="relative flex flex-col flex-1 md:gap-10 gap-6"
      >
        <GraphBar
          turbo
          Label={<GraphLabel label="Next.js 13" turbo />}
          duration={data.next13 * 1000}
          longestTime={roundedLongestTime}
          inView={graphInView}
        />

        <GraphBar
          Label={<GraphLabel label="Next.js 12" />}
          duration={data.next12 * 1000}
          longestTime={roundedLongestTime}
          inView={graphInView}
        />

        <GraphBar
          Label={<GraphLabel label="Vite" esbuild />}
          duration={data.vite * 1000}
          longestTime={roundedLongestTime}
          inView={graphInView}
        />

        <GraphBar
          Label={<GraphLabel label="Next.js 11" />}
          duration={data.next11 * 1000}
          longestTime={roundedLongestTime}
          inView={graphInView}
        />
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
}: {
  turbo?: boolean;
  duration: number;
  longestTime: number;
  Label: JSX.Element;
  inView?: boolean;
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
    <div className="md:flex-row md:flex w-full justify-center gap-1">
      <div className="w-48">{Label}</div>
      <div className="flex w-full items-center justify-between gap-4 z-10 border dark:border-[#333333] rounded-lg p-1">
        <motion.div
          animate={controls}
          variants={graphBarWrapperVariants}
          style={{ width: `${barWidth}%` }}
          transition={{ duration: 0.2 }}
          initial="hidden"
          className={cn(
            "flex items-center h-full rounded relative dark:bg-[#ffffff06] bg-[#00000006]"
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
            transition={{ duration: 0.2 }}
          />
        </motion.div>
        <motion.div
          animate={controls}
          variants={graphBarWrapperVariants}
          className="pr-2"
          transition={{ duration: 0.2 }}
        >
          <GraphTimer turbo={turbo} timer={timer} />
        </motion.div>
      </div>
    </div>
  );
}

const GraphTimer = ({ turbo, timer }: { turbo: boolean; timer: number }) => {
  return (
    <div className={`flex flex-row gap-2 w-24 justify-end items-center z-10`}>
      {turbo && (
        <div className="w-8 h-8 flex relative ">
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
            className="dark:opacity-60 opacity-0"
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
        <p
          className={cn(
            "font-space-grotesk m-0",
            gradients.benchmarkTurboLabel
          )}
        >
          turbo
        </p>
      )}
      {esbuild && (
        <p className="font-space-grotesk m-0 text-[#666666]">esbuild</p>
      )}
    </div>
  );
}
