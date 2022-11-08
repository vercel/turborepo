import type { FC, ReactElement } from "react";

import { Tabs as NextraTabs, Tab } from "nextra-theme-docs";
import useSWR from "swr";

export { Tab };

export const Tabs: FC<{
  storageKey?: string;
  items: string[];
  children: ReactElement;
}> = function ({ storageKey = "tab-index", items, children = null, ...props }) {
  // Use SWR so all tabs with the same key can sync their states.
  const { data, mutate } = useSWR(storageKey, (key) => {
    try {
      return JSON.parse(localStorage.getItem(key));
    } catch (e) {
      return null;
    }
  });

  const selectedIndex = items.indexOf(data);

  return (
    <NextraTabs
      onChange={(index) => {
        localStorage.setItem(storageKey, JSON.stringify(items[index]));
        mutate(items[index], false);
      }}
      selectedIndex={selectedIndex === -1 ? undefined : selectedIndex}
      items={items}
      {...props}
    >
      {children}
    </NextraTabs>
  );
};
