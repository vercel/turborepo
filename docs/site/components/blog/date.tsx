import type { ReactNode } from "react";

export function Date({
  children,
  update = undefined,
}: {
  children: ReactNode;
  update?: string;
}): JSX.Element {
  return (
    <div className="mt-2 text-sm text-gray-900 dark:text-gray-900">
      {children}
      {update !== undefined && (
        <div className="mt-1 text-center text-xs">Last updated {update}</div>
      )}
    </div>
  );
}
