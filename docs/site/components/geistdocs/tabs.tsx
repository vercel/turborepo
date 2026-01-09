"use client";

import type { ReactNode, ReactElement } from "react";
import { Children, cloneElement, isValidElement } from "react";
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
}): React.ReactElement {
  return (
    <FumaTabs id={storageKey} items={items} {...props}>
      {children}
    </FumaTabs>
  );
}

const packageManagers = ["pnpm", "yarn", "npm", "bun"];

const checkPackageManagerIndex = (index: number, provided: string) => {
  if (provided !== packageManagers[index]) {
    throw new Error(
      `Package manager at index ${index} must be ${packageManagers[index]}. Got ${provided}.`
    );
  }
};

interface TabProps {
  value: string;
  [key: string]: unknown;
}

interface TabElement extends ReactElement<TabProps> {
  props: TabProps;
}

/** Use <Tab /> component to create the tabs. */
export function PackageManagerTabs({
  children,
  ...props
}: {
  children: ReactNode;
}): React.ReactElement {
  // Filter to only valid React elements (ignoring whitespace/string children from MDX)
  const childElements = Children.toArray(children).filter(
    (child): child is TabElement => isValidElement(child)
  );

  if (packageManagers.length > childElements.length) {
    throw new Error(
      `Package manager tab is missing. Expected ${packageManagers.length} tabs, got ${childElements.length}.`
    );
  }

  childElements.forEach((packageManager, index) => {
    if (packageManager.props.value) {
      checkPackageManagerIndex(index, packageManager.props.value);
    } else {
      throw new Error(
        `Child at index ${index} is not a valid Tab element with a value prop.`
      );
    }
  });

  return (
    <FumaTabs
      groupId="package-manager"
      items={packageManagers}
      persist
      {...props}
    >
      {childElements.map((child, index) =>
        cloneElement(child, {
          ...child.props,
          value: packageManagers[index]
        })
      )}
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
}): React.ReactElement {
  const items = ["UNIX", "Windows"];

  // Filter to only valid React elements (ignoring whitespace/string children from MDX)
  const childElements = Children.toArray(children).filter(
    (child): child is TabElement => isValidElement(child)
  );

  if (items.length > childElements.length) {
    throw new Error(
      `Platform tab is missing. Expected ${items.length} tabs, got ${childElements.length}.`
    );
  }

  return (
    <FumaTabs groupId="platform-tabs" items={items} persist {...props}>
      {childElements.map((child, index) =>
        cloneElement(child, {
          ...child.props,
          value: items[index]
        })
      )}
    </FumaTabs>
  );
}
