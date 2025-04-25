"use client";

import { useTheme } from "next-themes";
import { useEffect, useState } from "react";
import { clsx } from "clsx";
import { DeviceDesktop } from "@/components/icons/device-desktop";
import { Moon } from "@/components/icons/moon";
import { Sun } from "@/components/icons/sun";
import styles from "./theme-switcher.module.css";

interface ThemeProviderValue {
  theme: string | undefined;
  setTheme: (theme: string) => void;
}

export function ThemeSwitcher({
  className,
  size = 28,
  short = false,
}: {
  className?: string;
  size?: number;
  short?: boolean;
}) {
  const { theme, setTheme } = useTheme() as ThemeProviderValue;

  const [mounted, setMounted] = useState(false);
  const iconSize = size / 2;
  const padding = size / 10.67;

  useEffect(() => {
    setMounted(true);
  }, []);

  // avoid hydration errors
  if (!mounted) return null;

  return (
    <div
      className={clsx(styles.root, className)}
      style={{ padding: short ? "0" : `${padding}px` }}
      role="radiogroup"
    >
      <label
        className={styles.switch}
        data-active={theme === "light"}
        style={{
          height: `${size}px`,
          width: `${size}px`,
        }}
        data-theme-switcher
      >
        <input
          type="radio"
          name="theme"
          value="light"
          checked={theme === "light"}
          onChange={() => {
            setTheme("light");
          }}
          className={styles.radioInput}
        />
        <Sun style={{ width: iconSize, height: iconSize }} />
      </label>
      <label
        className={styles.switch}
        data-active={theme === "system"}
        style={{
          height: `${size}px`,
          width: `${size}px`,
        }}
        data-theme-switcher
      >
        <input
          type="radio"
          name="theme"
          value="system"
          checked={theme === "system"}
          onChange={() => {
            setTheme("system");
          }}
          className={styles.radioInput}
        />
        <DeviceDesktop style={{ width: iconSize, height: iconSize }} />
      </label>
      <label
        className={styles.switch}
        data-active={theme === "dark"}
        style={{
          height: `${size}px`,
          width: `${size}px`,
        }}
        data-theme-switcher
      >
        <input
          type="radio"
          name="theme"
          value="dark"
          checked={theme === "dark"}
          onChange={() => {
            setTheme("dark");
          }}
          className={styles.radioInput}
        />
        <Moon style={{ width: iconSize, height: iconSize }} />
      </label>
    </div>
  );
}
