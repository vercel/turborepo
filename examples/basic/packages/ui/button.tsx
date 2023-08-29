"use client";

export function Button(): JSX.Element {
  return (
    <button
      onClick={(): void => {
        // eslint-disable-next-line no-alert -- alert is being used for demo purposes only
        alert("booped");
      }}
      type="button"
    >
      Boop
    </button>
  );
}
