import cn from "classnames";
import gradients from "./gradients.module.css";

export function CTAButton({
  children,
  outline,
}: {
  outline?: boolean;
  children: React.ReactNode;
}) {
  const outlineClasses =
    "border dark:border-neutral-400  dark:text-neutral-200 dark:hover:border-white dark:hover:text-white border-[#EAEAEA] text-neutral-800 hover:border-black hover:text-black";
  const filledClasses =
    "dark:text-black text-white border-transparent bg-black dark:bg-white";

  return (
    <div className="group relative w-full">
      <button
        className={`flex items-center justify-center w-full min-w-[120px] py-3 text-base font-medium  no-underline ${
          outline ? outlineClasses : filledClasses
        } rounded md:leading-6 transition-all duration-300`}
      >
        {children}
      </button>
      {!outline && (
        <div
          className={cn(
            "absolute bg-red-100 w-full h-full top-0 -z-10 rounded-full transition-all duration-300 blur-xl group-hover:opacity-70 opacity-0",
            gradients.translatingGlow
          )}
        />
      )}
    </div>
  );
}
