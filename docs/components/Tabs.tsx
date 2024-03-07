"use client";
import type { FC, ReactElement } from "react";
import { Tabs as FumaTabs, Tab } from "fumadocs-ui/components/tabs";

export { Tab };

export const Tabs: FC<{
  storageKey?: string;
  items: string[];
  children: ReactElement;
}> = function ({ storageKey, items, children = null, ...props }) {
  return (
    <FumaTabs id={storageKey} items={items} {...props}>
      {children}
    </FumaTabs>
  );
};

/** Use <Tab /> component to create the tabs. They will automatically be assigned their values in the order ["npm", "yarn", "pnpm"]. */
export const PackageManagerTabs: FC<{
  storageKey?: string;
  children: ReactElement;
}> = function ({ storageKey = "tab-index", children = null, ...props }) {
  const items = ["npm", "yarn", "pnpm"];

  return (
    <FumaTabs id={storageKey} items={items} {...props}>
      {/* @ts-expect-error */}
      {children.map((child, index) => {
        return { ...child, props: { ...child.props, value: items[index] } };
      })}
    </FumaTabs>
  );
};
