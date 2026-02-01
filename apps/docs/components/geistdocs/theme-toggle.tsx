"use client";

import { MoonIcon, SunIcon } from "lucide-react";
import { useTheme } from "next-themes";
import { useEffect, useState } from "react";
import { Button } from "../ui/button";

export const ThemeToggle = () => {
  const { resolvedTheme, setTheme } = useTheme();
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    setMounted(true);
  }, []);

  const handleClick = () => {
    setTheme(resolvedTheme === "dark" ? "light" : "dark");
  };

  if (!mounted) {
    return (
      <Button size="icon-sm" type="button" variant="ghost">
        <div className="size-4" />
      </Button>
    );
  }

  const Icon = resolvedTheme === "dark" ? MoonIcon : SunIcon;

  return (
    <Button onClick={handleClick} size="icon-sm" type="button" variant="ghost">
      <Icon className="size-4" />
    </Button>
  );
};
