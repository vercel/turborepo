"use client";

export function Button(): JSX.Element {
  return (
    // eslint-disable-next-line no-alert
    <button onClick={(): void => alert("booped")} type="button">
      Boop
    </button>
  );
}
