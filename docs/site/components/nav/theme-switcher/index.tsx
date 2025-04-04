"use client";

import { useTheme } from "next-themes";
import { useEffect, useState } from "react";
import { DeviceDesktop } from "@/components/icons/device-desktop";
import { Moon } from "@/components/icons/moon";
import { Sun } from "@/components/icons/sun";

export function ThemeSwitcher({
  size = 28,
  short = false,
}: {
  className?: string;
  size?: number;
  short?: boolean;
}) {
  const { theme, setTheme } = useTheme();

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
      className="w-fit flex rounded-full border-[1px] border-[var(--ds-gray-400)] md:ml-auto"
      style={{ padding: short ? "0" : `${padding}px` }}
      role="radiogroup"
    >
      <button
        aria-checked={theme === "light"}
        aria-label="Switch to light theme"
        className="flex items-center justify-center rounded-[inherit] border-0 text-[var(--ds-gray-900)] data-[active=true]:bg-[var(--ds-gray-300)] data-[active=true]:text-[var(--ds-gray-1000)]"
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
        className="flex items-center justify-center rounded-[inherit] border-0 text-[var(--ds-gray-900)] data-[active=true]:bg-[var(--ds-gray-300)] data-[active=true]:text-[var(--ds-gray-1000)]"
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
        className="flex items-center justify-center rounded-[inherit] border-0 text-[var(--ds-gray-900)] data-[active=true]:bg-[var(--ds-gray-300)] data-[active=true]:text-[var(--ds-gray-1000)]"
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
