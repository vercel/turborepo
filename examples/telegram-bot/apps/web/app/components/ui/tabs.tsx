"use client";

import { useState, useEffect } from "react";
import { motion } from "motion/react";
import { cn } from "../../lib/utils";

type Tab = {
  title: string | React.ReactNode;
  value: string;
  content?: string | React.ReactNode;
};

export const Tabs = ({
  tabs: propTabs,
  activeTabIndex,
  containerClassName,
  activeTabClassName,
  tabClassName,
  contentClassName,
  onTabChange,
}: {
  tabs: Tab[];
  activeTabIndex?: number;
  containerClassName?: string;
  activeTabClassName?: string;
  tabClassName?: string;
  contentClassName?: string;
  onTabChange?: (tab: Tab) => void;
}) => {
  // Use activeTabIndex prop or default to first tab
  const currentActiveIndex =
    activeTabIndex !== undefined && activeTabIndex >= 0 ? activeTabIndex : 0;
  const [active, setActive] = useState<Tab>(() => {
    if (!propTabs || propTabs.length === 0) {
      return { title: "", value: "", content: "" };
    }
    return propTabs[currentActiveIndex] || propTabs[0]!;
  });

  // Update active tab when activeTabIndex prop changes
  useEffect(() => {
    if (
      propTabs &&
      propTabs.length > 0 &&
      activeTabIndex !== undefined &&
      activeTabIndex >= 0 &&
      propTabs[activeTabIndex]
    ) {
      setActive(propTabs[activeTabIndex]!);
    }
  }, [activeTabIndex, propTabs]);

  // Add safety check for empty tabs array
  if (!propTabs || propTabs.length === 0) {
    return null;
  }

  const handleTabClick = (idx: number) => {
    const selectedTab = propTabs[idx];
    if (selectedTab) {
      setActive(selectedTab);
      onTabChange?.(selectedTab);
    }
  };

  return (
    <>
      <div
        className={cn(
          "flex flex-row items-center justify-start [perspective:1000px] relative overflow-auto sm:overflow-visible no-visible-scrollbar max-w-full w-full",
          containerClassName
        )}
      >
        {propTabs.map((tab, idx) => (
          <button
            key={typeof tab.title === "string" ? tab.title : tab.value}
            onClick={() => {
              handleTabClick(idx);
            }}
            className={cn("relative px-4 py-2 rounded-full", tabClassName)}
            style={{
              transformStyle: "preserve-3d",
            }}
          >
            {active.value === tab.value && (
              <motion.div
                layoutId="clickedbutton"
                transition={{ type: "spring", bounce: 0.3, duration: 0.6 }}
                className={cn(
                  "absolute inset-0 bg-gray-200 dark:bg-zinc-800 rounded-full ",
                  activeTabClassName
                )}
              />
            )}

            <span className="relative block text-black dark:text-white">
              {tab.title}
            </span>
          </button>
        ))}
      </div>
      {!contentClassName?.includes("hidden") && (
        <FadeInDiv
          active={active}
          key={active.value}
          className={cn("mt-32", contentClassName)}
        />
      )}
    </>
  );
};

export const FadeInDiv = ({
  className,
  active,
}: {
  className?: string;
  key?: string;
  active: Tab;
  hovering?: boolean;
}) => {
  return (
    <div className="relative w-full h-full">
      <div className={cn("w-full h-full", className)}>{active.content}</div>
    </div>
  );
};
