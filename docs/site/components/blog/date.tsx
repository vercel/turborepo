import type { ReactNode } from "react";

// biome-ignore lint/suspicious/noShadowRestrictedNames: Ignored using `--suppress`
export function Date({
  children,
  update = undefined,
}: {
  children: ReactNode;
  update?: string;
}): JSX.Element {
  return (
    <div className="mt-2 text-center text-sm text-gray-500 dark:text-gray-400">
      {children}

      {update !== undefined && (
        <div className="mt-1 text-center text-xs">Last updated {update}</div>
      )}
    </div>
  );
}
