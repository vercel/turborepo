export function Gradient({
  conic,
  className,
  small,
}: {
  small?: boolean;
  conic?: boolean;
  className?: string;
}) {
  return (
    <span
      className={`absolute mix-blend-normal will-change-[filter] rounded-[100%] ${
        small ? "blur-[32px]" : "blur-[75px]"
      } ${
        conic
          ? "bg-[conic-gradient(from_180deg_at_50%_50%,_#2a8af6_0deg,_#a853ba_180deg,_#e92a67_360deg)]"
          : ""
      } ${className ?? ""}`}
    />
  );
}
