import type { MouseEvent } from "react";
import { useEffect, useCallback, useState, useRef } from "react";
import { useTheme } from "nextra-theme-docs";
import Link from "next/link";
import classNames from "classnames";
import { useTurboSite } from "../SiteSwitcher";
import { VercelLogo } from "./icons";
import { PRODUCT_MENU_ITEMS, PLATFORM_MENU_ITEMS } from "./items";
import type { MenuItemProps } from "./types";

function MenuDivider({ children, ...other }: { children: string }) {
  return (
    <h3
      className={classNames(
        "group flex items-center px-4 py-2 text-xs dark:text-gray-600 text-gray-500 font-bold"
      )}
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
}: MenuItemProps) {
  const [copied, setCopied] = useState(false);

  const handleClick = () => {
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

  const classes = classNames(
    className,
    "group flex items-center px-4 py-2 text-sm dark:hover:bg-gray-800 hover:bg-gray-200 w-full rounded-md"
  );

  if (type === "internal") {
    return (
      // eslint-disable-next-line @typescript-eslint/no-non-null-assertion -- Going to allow it here...but it's not truly correct.
      <Link className={classes} href={href!} onClick={handleClick} {...other}>
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

export function LogoContext() {
  const [open, setOpen] = useState(false);
  // By default, the repo logo is used.
  const site = useTurboSite() || "repo";
  const menu = useRef<HTMLDivElement | null>(null);
  const { theme = "dark" } = useTheme();

  const toggleMenu = (e: MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    if (e.type === "contextmenu") {
      setOpen((prev) => !prev);
    } else {
      setOpen(false);
      window.open(`https://vercel.com`, "_blank");
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
    <div className="block relative">
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
          className="absolute border dark:border-gray-700 left-6 z-10 mt-2 w-60 origin-top-right divide-y divide-gray-100 rounded-md bg-white dark:bg-black shadow-sm focus:outline-none"
          ref={menu}
        >
          <div className="p-2">
            <MenuDivider>Platform</MenuDivider>
            {PLATFORM_MENU_ITEMS({ theme, site }).map((item) => (
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
            {PRODUCT_MENU_ITEMS({ theme, site }).map((item) => (
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
