import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { BenchmarkCategory } from "./PackBenchmarks";
import classNames from "classnames";
import gradients from "./gradients.module.css";

const TABS: {
  id: BenchmarkCategory;
  title: string;
  soon: boolean;
}[] = [
  {
    id: "cold",
    title: "Cold Start",
    soon: false,
  },
  {
    id: "from_cache",
    title: "Start from Cache",
    soon: false,
  },
  {
    id: "file_change",
    title: "File Change",
    soon: false,
  },
  {
    id: "code_build",
    title: "Code Build",
    soon: true,
  },
  {
    id: "build_from_cache",
    title: "Build from Cache",
    soon: true,
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
    <div className="flex w-full items-center justify-center">
      <div className="relative overflow-x-auto flex items-center justify-start no-scrollbar">
        <AnimatePresence>
          <div className="flex flex-row items-center rounded-full p-1 dark:bg-[#ffffff03] bg-[#00000005] mx-6">
            {TABS.map((tab, index) => (
              <div
                className="py-3 px-5 relative"
                key={tab.id}
                onClick={() => onTabClick(index)}
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
                <motion.div
                  animate={{ opacity: activeTab === index ? 1 : 0.4 }}
                  className="flex flex-row items-center gap-2 justify-center whitespace-nowrap"
                  transition={{ ...TRANSITION, duration: 0.2 }}
                  style={{ cursor: tab.soon ? "not-allowed" : "pointer" }}
                >
                  <p
                    className="font-medium m-0 z-10"
                    style={{ opacity: tab.soon ? 0.6 : 1 }}
                  >
                    {tab.title}
                  </p>
                  {tab.soon && <SoonBadge />}
                </motion.div>
              </div>
            ))}
          </div>
        </AnimatePresence>
      </div>
    </div>
  );
}
