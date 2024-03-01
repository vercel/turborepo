/* eslint-disable @typescript-eslint/no-floating-promises --  Lots of SWR and local storage. Not worth fixing so we'll ignore. */
/* eslint-disable @typescript-eslint/no-unsafe-argument --  Lots of SWR and local storage. Not worth fixing so we'll ignore. */
/* eslint-disable @typescript-eslint/no-non-null-assertion --  Lots of SWR and local storage. Not worth fixing so we'll ignore. */
/* eslint-disable @typescript-eslint/no-unsafe-return --  Lots of SWR and local storage. Not worth fixing so we'll ignore. */
/* eslint-disable @typescript-eslint/no-unsafe-assignment --  Lots of SWR and local storage. Not worth fixing so we'll ignore. */
/* eslint-disable react/function-component-definition --  Lots of SWR and local storage. Not worth fixing so we'll ignore. */

"use client";
import type { FC, ReactElement } from "react";
import { Tabs as FumaTabs, Tab } from "fumadocs-ui/components/tabs";
import { CodeBlock } from "fumadocs-ui/components/codeblock";

export { Tab };

export const Thing = () => {
  return <CodeBlock lang="bash">npm install fumadocs-ui</CodeBlock>;
};

export const Tabs: FC<{
  storageKey?: string;
  items: string[];
  children: ReactElement;
}> = function ({ storageKey = "tab-index", items, children = null, ...props }) {
  return (
    <FumaTabs id={storageKey} items={items} {...props}>
      {children}
    </FumaTabs>
  );
};

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
