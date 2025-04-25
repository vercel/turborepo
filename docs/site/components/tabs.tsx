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

interface TabElement extends ReactElement {
  props: {
    value: string;
    [key: string]: unknown;
  };
}

/** Use <Tab /> component to create the tabs. */
export function PackageManagerTabs({
  children,
  ...props
}: {
  children: ReactNode;
}): JSX.Element {
  // Validate that children is an array of valid elements
  if (!Array.isArray(children)) {
    throw new Error("Children must be an array.");
  }

  const childElements = children as Array<TabElement>;

  if (packageManagers.length > childElements.length) {
    throw new Error("Package manager tab is missing a value.");
  }

  childElements.forEach((packageManager, index) => {
    if (isValidElement(packageManager) && packageManager.props.value) {
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
      {childElements.map((child, index) => {
        if (!isValidElement(child)) {
          return null;
        }
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
      {/*
        React's Children.map and cloneElement have limitations in TypeScript's type system.
        This is a standard React pattern for modifying props on child elements.
      */}
      {/* eslint-disable @typescript-eslint/no-unsafe-argument, @typescript-eslint/no-unsafe-return -- React's Children and cloneElement APIs don't have proper TypeScript typings, but this is a safe pattern */}
      {Children.map(children, (child, index) => {
        if (!isValidElement(child)) {
          return child;
        }

        // Create a new props object with the items value
        // We need to use Record<string, unknown> here, but TypeScript still complains
        // about spreading, so we need to cast to a more general type
        const newProps = {
          ...(child.props as Record<string, unknown>),
          value: items[index],
        };

        return cloneElement(child, newProps);
      })}
      {/* eslint-enable -- Re-enable ESLint rules after the React Children manipulation section */}
    </FumaTabs>
  );
}
