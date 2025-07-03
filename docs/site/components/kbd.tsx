"use client";
import { clsx } from "clsx";
import { useEffect, useState } from "react";
import type { JSX } from "react";
import { isApple } from "./utils";
import styles from "./kbd.module.css";

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
  small,
  children,
  className,
  ...props
}: KbdProps): JSX.Element {
  return (
    <kbd
      className={clsx(styles.kbd, { [String(styles.small)]: small }, className)}
      {...props}
    >
      {meta ? <Meta /> : null}
      {shift ? <span>⇧</span> : null}
      {alt ? <span>⌥</span> : null}
      {ctrl ? <span>⌃</span> : null}
      {children ? <span>{children}</span> : null}
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
    <span style={{ minWidth: "1em", display: "inline-block" }}>{label}</span>
  );
}
