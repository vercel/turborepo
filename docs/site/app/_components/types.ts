import type { ReactNode } from "react";

type MenuItemType = "internal" | "external" | "copy";

export interface MenuItemProps extends ContextItem {
  closeMenu?: () => void;
  className?: string;
}

export interface ContextList {
  theme: string;
}

export interface ContextItem {
  name: string;
  "aria-label": string;
  disabled?: boolean;
  type: MenuItemType;
  children: ReactNode;
  prefix: ReactNode;
  href?: string;
  onClick?: () => void;
}
