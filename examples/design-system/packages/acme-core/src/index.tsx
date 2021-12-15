import * as React from "react";

export const Button = () => {
  const [count, setCount] = React.useState(0);
  return (
    <div
      style={{
        background: `rgba(255,255,255,.05)`,
        borderRadius: `8px`,
        padding: 16,
      }}
    >
      <p>
        This is component iadasdfsdfdffsfsds from <code>ui</code>
      </p>
      <p>
        <button type="button" onClick={() => setCount((c) => c + 1)}>
          count {count}
        </button>
      </p>
    </div>
  );
};
