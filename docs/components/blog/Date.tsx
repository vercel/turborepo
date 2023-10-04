import { ReactNode } from "react";

function Date({
  children,
  update = null,
}: {
  children: ReactNode;
  update?: string;
}) {
  return (
    <div className="text-sm mt-2 text-center text-gray-500 dark:text-gray-400 font-space-grotesk">
      {children}

      {update != null && (
        <div className="text-xs mt-1 text-center">Last updated {update}</div>
      )}
    </div>
  );
}

export default Date;
