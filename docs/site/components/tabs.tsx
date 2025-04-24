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
  items: Array<string>;
  children: ReactNode;
}): JSX.Element {
  return (
    <FumaTabs id={storageKey} items={items} {...props}>
      {children}
    </FumaTabs>
  );
}

const packageManagers = ["pnpm", "yarn", "npm", "bun (Beta)"];

const checkPackageManagerIndex = (index: number, provided: string) => {
  if (provided !== packageManagers[index]) {
    throw new Error(
      `Package manager at index ${index} must be ${packageManagers[index]}.`
    );
  }
};

/** Use <Tab /> component to create the tabs. */
export function PackageManagerTabs({
  children,
  ...props
}: {
  children: ReactNode;
}): JSX.Element {
  if (!Array.isArray(children)) {
    throw new Error("Children must be an array.");
  }

  if (packageManagers.length > children.length) {
    throw new Error(`Package manager tab is missing a value.`);
  }

  children.forEach((packageManager, index) => {
    checkPackageManagerIndex(index, packageManager.props.value);
  });

  return (
    <FumaTabs
      groupId="package-manager"
      items={packageManagers}
      persist
      {...props}
    >
      {children.map((child, index) => {
        return {
          ...child,
          props: {
            ...child.props,
            value: packageManagers[index],
          },
        };
      })}
    </FumaTabs>
  );
}

/** Use <Tab /> component to create the tabs. They will automatically be assigned their values in the order ["UNIX", "Windows"]. */
export function PlatformTabs({
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
    <FumaTabs groupId="platform-tabs" items={items} persist {...props}>
      {Children.map(children, (child, index) =>
        // eslint-disable-next-line @typescript-eslint/no-unsafe-argument, @typescript-eslint/no-unsafe-member-access
        cloneElement(child, { ...child.props, value: items[index] })
      )}
    </FumaTabs>
  );
}
