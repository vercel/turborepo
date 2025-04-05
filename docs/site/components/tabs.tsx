"use client";

import type { ReactNode } from "react";
import { Children, cloneElement } from "react";
import { Tabs as FumaTabs, Tab } from "fumadocs-ui/components/tabs";

export { Tab };
export function Tabs({
  storageKey,
  items,
  children,
  ...props
}: {
  storageKey?: string;
  items: string[];
  children: ReactNode;
}): JSX.Element {
  return (
    <FumaTabs id={storageKey} items={items} {...props}>
      {children}
    </FumaTabs>
  );
}

const packageManagers = ["pnpm", "yarn", "npm"];

const checkPackageManagerIndex = (index: number, provided: string) => {
  if (provided !== packageManagers[index]) {
    throw new Error(
      `Package manager at index ${index} must be ${packageManagers[index]}.`
    );
  }
};

/** Use <Tab /> component to create the tabs. They will automatically be assigned their values in the order ["npm", "yarn", "pnpm"]. */
export function PackageManagerTabs({
  storageKey = "package-manager-tabs",
  children,
  ...props
}: {
  storageKey?: string;
  children: ReactNode;
}): JSX.Element {
  if (!Array.isArray(children)) {
    throw new Error("Children must be an array.");
  }

  children.forEach((packageManager, index) => {
    if (!packageManager.props.value) {
      throw new Error(`Package manager tab is missing a value.`);
    }

    checkPackageManagerIndex(index, packageManager.props.value);
  });

  return (
    <FumaTabs id={storageKey} items={packageManagers} {...props}>
      {children.map((child, index) => {
        // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access, @typescript-eslint/no-unsafe-return, @typescript-eslint/no-unsafe-assignment
        return {
          ...child,
          props: { ...child.props, value: packageManagers[index] },
        };
      })}
    </FumaTabs>
  );
}

/** Use <Tab /> component to create the tabs. They will automatically be assigned their values in the order ["UNIX", "Windows"]. */
export function PlatformTabs({
  storageKey = "platform-tabs",
  children,
  ...props
}: {
  storageKey?: string;
  children: ReactNode;
}): JSX.Element {
  const items = ["UNIX", "Windows"];

  if (!Array.isArray(children)) {
    throw new Error("Children must be an array.");
  }

  return (
    <FumaTabs id={storageKey} items={items} {...props}>
      {Children.map(children, (child, index) =>
        // eslint-disable-next-line @typescript-eslint/no-unsafe-argument, @typescript-eslint/no-unsafe-member-access
        cloneElement(child, { ...child.props, value: items[index] })
      )}
    </FumaTabs>
  );
}
