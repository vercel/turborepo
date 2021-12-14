import * as React from "react";

export interface CardProps {
  children: React.ReactNode;
}

export function Card(props: CardProps) {
  return (
    <div
      style={{
        border: "1px solid #ccc",
        padding: "1rem",
        borderRadius: "0.5rem",
      }}
    >
      {props.children}
    </div>
  );
}

Card.displayName = "Card";
