import { cn } from "#components/cn.ts";

export const ForwardSlash = ({ className }: { className?: string }) => {
  return (
    <svg
      className={cn("ml-1 mr-1 text-[#eaeaea] dark:text-[#333]", className)}
      fill="none"
      height={24}
      shapeRendering="geometricPrecision"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.5"
      viewBox="0 0 24 24"
    >
      <path d="M16.88 3.549L7.12 20.451" />
    </svg>
  );
};
