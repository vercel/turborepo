import * as React from "react";

export const CounterButton = () => {
  const [count, setCount] = React.useState(0);
  return (
    <div
      style={{
        background: `rgba(0,0,0,0.05)`,
        borderRadius: `8px`,
        padding: "1.5rem",
        fontWeight: 500,
      }}
    >
      <p style={{ margin: "0 0 1.5rem 0" }}>
        This component is from{" "}
        <code
          style={{
            padding: "0.2rem 0.3rem",
            background: `rgba(0,0,0,0.1)`,
            borderRadius: "0.25rem",
          }}
        >
          ui
        </code>
      </p>
      <div>
        <button
          style={{
            background: "black",
            color: "white",
            border: "none",
            padding: "0.5rem 1rem",
            borderRadius: "0.25rem",
            display: "inline-block",
            cursor: "pointer",
          }}
          type="button"
          onClick={() => setCount((c) => c + 1)}
        >
          Count: {count}
        </button>
      </div>
    </div>
  );
};
