import cn from "classnames";
import styles from "./turbohero-background.module.css";

export function TurboheroBackground(): JSX.Element {
  return (
    <div
      className={cn(
        "![perspective:1000px] sm:![perspective:1000px] md:![perspective:1000px] lg:![perspective:1000px]",
        styles.container
      )}
    >
      <div
        className="z-[100] absolute inset-0 [--gradient-stop-1:0px] [--gradient-stop-2:50%]"
        style={{
          background:
            "linear-gradient(to top, rgba(0,0,0,0) 0px, var(--geist-foreground) 50%)",
        }}
      />
      <div
        style={{
          transform: "rotateX(75deg)",
          position: "absolute",
          top: 0,
          bottom: 0,
          left: 0,
          right: 0,
        }}
      >
        <div className={styles.lines} />
      </div>
    </div>
  );
}
