"use client";

import { useTheme } from "next-themes";
import { useEffect, useState } from "react";
import { clsx } from "clsx";
import { Moon } from "#components/icons/moon.tsx";
import { Sun } from "#components/icons/sun.tsx";
import { DeviceDesktop } from "#components/icons/device-desktop.tsx";
import styles from "./theme-switcher.module.css";

interface ThemeProvider {
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
  const { theme, setTheme } = useTheme() as ThemeProvider;

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
      <button
        aria-checked={theme === "light"}
        aria-label="Switch to light theme"
        className={styles.switch}
        data-active={theme === "light"}
        style={{
          height: `${size}px`,
          width: `${size}px`,
        }}
        data-theme-switcher
        onClick={(): void => {
          setTheme("light");
        }}
        role="radio"
        type="button"
      >
        <Sun style={{ width: iconSize, height: iconSize }} />
      </button>
      <button
        aria-checked={theme === "system"}
        aria-label="Switch to system theme"
        className={styles.switch}
        style={{
          height: `${size}px`,
          width: `${size}px`,
        }}
        data-active={theme === "system"}
        data-theme-switcher
        onClick={(): void => {
          setTheme("system");
        }}
        role="radio"
        type="button"
      >
        <DeviceDesktop style={{ width: iconSize, height: iconSize }} />
      </button>
      <button
        aria-checked={theme === "dark"}
        aria-label="Switch to dark theme"
        className={styles.switch}
        style={{
          height: `${size}px`,
          width: `${size}px`,
        }}
        data-active={theme === "dark"}
        data-theme-switcher
        onClick={(): void => {
          setTheme("dark");
        }}
        role="radio"
        type="button"
      >
        <Moon style={{ width: iconSize, height: iconSize }} />
      </button>
    </div>
  );
}
