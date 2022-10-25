import { useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { BenchmarkCategory } from "./PackBenchmarks";
import classNames from "classnames";
import gradients from "../home-shared/gradients.module.css";

const TABS: {
  id: BenchmarkCategory;
  title: string;
  soon: boolean;
  tooltip: string;
}[] = [
  {
    id: "cold",
    title: "Cold Start",
    soon: false,
    tooltip: "First run",
  },
  {
    id: "file_change",
    title: "File Change",
    soon: false,
    tooltip: "Hot Reload (HMR)",
  },
  {
    id: "code_build",
    title: "Code Build",
    soon: true,
    tooltip: "First Build",
  },
  {
    id: "build_from_cache",
    title: "Build from Cache",
    soon: true,
    tooltip: "Second Build",
  },
];

const TRANSITION = {
  duration: 0.3,
  ease: [0.59, 0.15, 0.18, 0.93],
};

function SoonBadge() {
  return (
    <span className="inline-flex items-center h-5 px-2 rounded-full text-xs font-medium dark:text-[#888888] dark:bg-[#333333] text-[#666666] bg-[#EAEAEA] ">
      Soon
    </span>
  );
}

export function PackBenchmarkTabs({
  onTabChange,
}: {
  onTabChange: (tab: BenchmarkCategory) => void;
}) {
  const [activeTab, setActiveTab] = useState(0);

  const onTabClick = (index: number) => {
    if (TABS[index].soon) return;
    setActiveTab(index);
    onTabChange(TABS[index].id);
  };

  return (
    <div className="flex items-center justify-center w-full">
      <div className="relative flex items-center justify-start pb-12 overflow-x-scroll overflow-y-clip no-scrollbar">
        <AnimatePresence>
          <div className="flex flex-row items-center rounded-full p-1 dark:bg-[#ffffff0d] bg-[#00000005] mx-5">
            {TABS.map((tab, index) => (
              <button
                className="relative px-5 py-3"
                key={tab.id}
                onClick={() => onTabClick(index)}
                disabled={tab.soon}
              >
                {TABS[activeTab].id === tab.id && (
                  <motion.div
                    className={classNames(
                      gradients.benchmarkActiveTab,
                      "absolute w-full rounded-full h-full top-0 left-0 border border-neutral-200 dark:border-neutral-800"
                    )}
                    layoutId="tabSwitcher"
                    style={{ borderRadius: 36 }}
                    transition={TRANSITION}
                  />
                )}
                <ToolTip text={tab.tooltip}>
                  <motion.div
                    animate={{ opacity: activeTab === index ? 1 : 0.4 }}
                    className="flex flex-row items-center justify-center gap-2 whitespace-nowrap"
                    transition={{ ...TRANSITION, duration: 0.2 }}
                    style={{ cursor: tab.soon ? "not-allowed" : "pointer" }}
                  >
                    <span
                      className="z-10 m-0 font-medium"
                      style={{ opacity: tab.soon ? 0.6 : 1 }}
                    >
                      {tab.title}
                    </span>
                    {tab.soon && <SoonBadge />}
                  </motion.div>
                </ToolTip>
              </button>
            ))}
          </div>
        </AnimatePresence>
      </div>
    </div>
  );
}

function ToolTip({ text, children }: { text; children: React.ReactNode }) {
  const [show, setShow] = useState(false);
  const timeout = useRef<NodeJS.Timeout>();

  const onMouseEnter = () => {
    timeout.current = setTimeout(() => {
      setShow(true);
    }, 800);
  };

  const onMouseLeave = () => {
    clearTimeout(timeout.current);
    setShow(false);
  };

  return (
    <div
      className="relative"
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
    >
      <motion.div
        animate={show ? { opacity: 1, y: 0 } : { opacity: 0, y: -4 }}
        transition={{ duration: 0.2, ease: [0.59, 0.15, 0.18, 0.93] }}
        className={
          "absolute top-[100%] mt-4 w-full flex flex-col items-center justify-center z-50"
        }
      >
        <div className={gradients.tooltipArrow} />
        <div className="dark:bg-[#333333] bg-neutral-100 rounded-lg px-4 py-1 whitespace-nowrap">
          <p className="font-sans text-sm text-[#888888]">{text}</p>
        </div>
      </motion.div>
      <div>{children}</div>
    </div>
  );
}
