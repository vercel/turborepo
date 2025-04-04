"use client";
import { useEffect, useState } from "react";
import type { JSX } from "react";
import { isApple } from "./utils";

interface KbdProps {
  meta?: boolean;
  shift?: boolean;
  alt?: boolean;
  ctrl?: boolean;
  small?: boolean;
  className?: string;
  children?: React.ReactNode;
  style?: React.CSSProperties;
}

export function Kbd({
  meta,
  shift,
  alt,
  ctrl,
  children,
  className,
  ...props
}: KbdProps): JSX.Element {
  return (
    <kbd
      className="bg-gray-100 text-gray-100 inline-block text-center min-w-[var(--geist-gap)] text-sm leading-[1.7em] py-[6px] rounded-[4px] font-sans ml-[4px] min-h-[24px]"
      {...props}
    >
      {meta ? <Meta /> : null}
      {shift ? <span className="ml-[4px]">⇧</span> : null}
      {alt ? <span className="ml-[4px]">⌥</span> : null}
      {ctrl ? <span className="ml-[4px]">⌃</span> : null}
      {children ? <span className="ml-[4px]">{children}</span> : null}
    </kbd>
  );
}

function Meta(): JSX.Element {
  // u00A0 = &nbsp;
  const [label, setLabel] = useState("\u00A0");

  const apple = isApple();
  useEffect(() => {
    // Meta is Command on Apple devices, otherwise Control
    if (apple === true) {
      setLabel("⌘");
    }

    // Explicitly say "Ctrl" instead of the symbol "⌃"
    // because most Windows/Linux laptops do not print the symbol
    // Other keyboard-intensive apps like Linear do this
    if (apple === false) {
      setLabel("Ctrl");
    }
  }, [apple]);

  return (
    <span
      className="ml-[4px]"
      style={{ minWidth: "1em", display: "inline-block" }}
    >
      {label}
    </span>
  );
}
