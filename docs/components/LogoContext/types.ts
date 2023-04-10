import type { ReactNode } from "react";
import { TurboSite } from "../SiteSwitcher";

type MenuItemType = "internal" | "external" | "copy";

export interface MenuItemProps extends ContextItem {
  closeMenu?: () => void;
  className?: string;
}

export interface ContextList {
  theme: string;
  site: TurboSite;
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
