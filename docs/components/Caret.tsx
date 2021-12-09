import { memo } from "react";
import styles from "./caret.module.css";
import cn from "classnames";

interface CaretProps {
  mini?: boolean;
  blink?: boolean;
}
export const Caret = memo(function Caret({ mini, blink }: CaretProps) {
  return (
    <span
      className={cn(styles.caret, {
        [styles.mini]: mini,
        [styles.blink]: blink,
      })}
    />
  );
});

export const Prompt = memo(function Prompt({ children = "my-site/" }) {
  return (
    <span className={styles.prompt}>
      <span className={styles.triangle}>▲</span> {children}{" "}
    </span>
  );
});
