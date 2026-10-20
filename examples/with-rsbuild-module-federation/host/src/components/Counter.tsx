import { useState } from "react";

export default () => {
  const [count, setCount] = useState<number>(0);

  return (
    <button
      style={{
        border: "0 solid #e2e8f0",
        marginTop: "12px",
        backgroundColor: "#f6b352",
        borderRadius: "6px",
        fontWeight: "700",
        padding: ".5rem 1rem .5rem 1rem",
        color: "#191611",
      }}
      onClick={() => setCount(count + 1)}
    >
      Host counter: {count}
    </button>
  );
};
