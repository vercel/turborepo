import "./button.css";
import "./button.json" with { type: "json" };

export const Button = ({ children }: { children: React.ReactNode }) => {
  return <button>{children}</button>;
};
