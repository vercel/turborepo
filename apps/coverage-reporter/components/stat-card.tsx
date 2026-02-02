interface StatCardProps {
  label: string;
  value: string | number;
  delta?: number;
  suffix?: string;
}

export function StatCard({ label, value, delta, suffix = "%" }: StatCardProps) {
  const formatDelta = (d: number): string => {
    const sign = d >= 0 ? "+" : "";
    return `${sign}${d.toFixed(2)}${suffix}`;
  };

  return (
    <div className="stat">
      <div className="stat-value">
        {typeof value === "number" ? value.toFixed(1) : value}
        {suffix && <span style={{ fontSize: "1rem" }}>{suffix}</span>}
      </div>
      <div className="stat-label">{label}</div>
      {delta !== undefined && (
        <div className={`stat-delta ${delta >= 0 ? "positive" : "negative"}`}>
          {formatDelta(delta)}
        </div>
      )}
    </div>
  );
}
