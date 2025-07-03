"use client";

import type { MouseEvent } from "react";
import { useEffect, useCallback, useState, useRef } from "react";
import Link from "next/link";
import { cn } from "#components/cn.ts";
import { VercelLogo } from "./logos";
import { PRODUCT_MENU_ITEMS, PLATFORM_MENU_ITEMS } from "./items";
import type { MenuItemProps } from "./types";
import { useColorScheme } from "./use-color-scheme";

function MenuDivider({
  children,
  ...other
}: {
  children: string;
}): JSX.Element {
  return (
    <h3
      className="group flex items-center px-4 py-2 text-xs font-bold text-gray-500 dark:text-gray-600"
      {...other}
    >
      {children}
    </h3>
  );
}

function MenuItem({
  children,
  prefix,
  className,
  type,
  href,
  onClick,
  closeMenu,
  disabled,
  ...other
}: MenuItemProps): JSX.Element {
  const [copied, setCopied] = useState(false);

  const handleClick = (): void => {
    if (onClick) {
      onClick();
    }
    if (type === "copy") {
      setCopied(true);
    } else {
      closeMenu?.();
    }
  };

  useEffect(() => {
    if (copied) {
      const timeout = setTimeout(() => {
        setCopied(false);
        closeMenu?.();
      }, 2000);
      return () => {
        clearTimeout(timeout);
      };
    }
  }, [copied, closeMenu]);

  const classes = cn(
    className,
    "group flex items-center px-4 py-2 text-sm dark:hover:bg-gray-800 hover:bg-gray-200 w-full rounded-md"
  );

  if (type === "internal") {
    return (
      <Link className={classes} href={href} onClick={handleClick} {...other}>
        {prefix}
        {children}
      </Link>
    );
  }
  if (type === "external") {
    return (
      <a
        className={classes}
        href={href}
        onClick={handleClick}
        rel="noopener noreferrer"
        target="_blank"
        {...other}
      >
        {prefix}
        {children}
      </a>
    );
  }

  // Copy button
  return (
    <button
      className={classes}
      disabled={disabled}
      onClick={handleClick}
      type="button"
      {...other}
    >
      {prefix}
      {copied ? "Copied to clipboard!" : children}
    </button>
  );
}

export function LogoContext(): JSX.Element {
  const [open, setOpen] = useState(false);
  // By default, the repo logo is used.
  const menu = useRef<HTMLDivElement | null>(null);
  const theme = useColorScheme();

  const toggleMenu = (e: MouseEvent<HTMLButtonElement>): void => {
    e.preventDefault();
    if (e.type === "contextmenu") {
      setOpen((prev) => !prev);
    } else {
      setOpen(false);
      window.open(`https://vercel.com`, "_blank", "noopener");
    }
  };

  const onClickOutside: EventListener = useCallback(
    (e) => {
      // @ts-expect-error -- Event listener typing is weird.
      if (menu.current && open && !menu.current.contains(e.target)) {
        setOpen(false);
      }
    },
    [open]
  );

  useEffect(() => {
    document.addEventListener("click", onClickOutside, true);
    return () => {
      document.removeEventListener("click", onClickOutside, true);
    };
  }, [onClickOutside]);

  return (
    <div className="relative block">
      <button
        className="flex"
        onClick={toggleMenu}
        onContextMenu={toggleMenu}
        type="button"
      >
        <VercelLogo />
      </button>
      {open ? (
        <div
          className="absolute left-6 z-10 mt-2 w-60 origin-top-right divide-y divide-gray-100 rounded-md border bg-white shadow-sm focus:outline-none dark:border-gray-700 dark:bg-black"
          ref={menu}
        >
          <div className="p-2">
            <MenuDivider>Platform</MenuDivider>
            {PLATFORM_MENU_ITEMS({ theme }).map((item) => (
              <MenuItem
                closeMenu={() => {
                  setOpen(false);
                }}
                key={item.name}
                {...item}
              >
                {item.children}
              </MenuItem>
            ))}
            <MenuDivider>Products</MenuDivider>
            {PRODUCT_MENU_ITEMS().map((item) => (
              <MenuItem
                closeMenu={() => {
                  setOpen(false);
                }}
                key={item.name}
                {...item}
              >
                {item.children}
              </MenuItem>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}
