import { ReactNode } from "react";

function Date({ children }: { children: ReactNode }) {
  return (
    <div className="text-sm mt-2 text-center text-gray-500 dark:text-gray-400 font-space-grotesk">
      {children}
    </div>
  );
}

export default Date;
