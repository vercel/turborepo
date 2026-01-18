interface ProgressBarProps {
  percent: number;
  size?: "sm" | "md";
}

export function ProgressBar({ percent, size = "md" }: ProgressBarProps) {
  const getColorClass = (p: number): string => {
    if (p >= 80) return "high";
    if (p >= 50) return "medium";
    return "low";
  };

  const height = size === "sm" ? "4px" : "8px";

  return (
    <div className="progress-bar" style={{ height }}>
      <div
        className={`progress-bar-fill ${getColorClass(percent)}`}
        style={{ width: `${Math.min(100, percent)}%` }}
      />
    </div>
  );
}
