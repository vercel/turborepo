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
    "border dark:border-[#333333] dark:text-neutral-400 dark:hover:border-white dark:hover:text-white border-[#EAEAEA] text-neutral-800 hover:border-black hover:text-black";
  const filledClasses = "dark:text-black text-white border-transparent";

  return (
    <div className="group relative w-full">
      <button
        className={cn(
          { [gradients.buttonGradient]: !outline },
          `flex items-center justify-center w-full min-w-[120px] py-3 text-base font-medium  no-underline ${
            outline ? outlineClasses : filledClasses
          } rounded-full md:leading-6 cursor-pointer transition-all duration-300`
        )}
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
