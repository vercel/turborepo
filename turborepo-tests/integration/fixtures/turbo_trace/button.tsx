import "./button.css";
import "./button.json";

export const Button = ({ children }: { children: React.ReactNode }) => {
  return <button>{children}</button>;
};
