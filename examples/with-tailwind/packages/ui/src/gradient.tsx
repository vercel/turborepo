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
      className={`ui:absolute ui:mix-blend-normal ui:will-change-[filter] ui:rounded-[100%] ${
        small ? "ui:blur-[32px]" : "ui:blur-[75px]"
      } ${
        conic
          ? "ui:bg-gradient-to-r ui:bg-red-1000 ui:from-10% ui:via-purple-1000 ui:via-30% ui:to-blue-1000 ui:to-100%"
          : ""
      } ${className ?? ""}`}
    />
  );
}
