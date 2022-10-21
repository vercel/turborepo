import { useLayoutEffect, useState } from "react";
import useMeasure from "react-use-measure";
import type { HTMLAttributes } from "react";
import styles from "./turbohero-background.module.css";
import cn from "classnames";
import useIsomorphicLayoutEffect from "../../useIsomorphicLayoutEffect";

const GRID_SIZE = 80;

const LINE_WIDTH = 2;
type LineProps = (HTMLAttributes<HTMLSpanElement> & { key: string })[];

export function TurboheroBackground(): JSX.Element {
  const [contentRef, { width }] = useMeasure();
  const [verticalLineProps, setVerticalLineProps] = useState<LineProps>([]);
  useIsomorphicLayoutEffect(() => {
    const props: LineProps = [];
    const gridSize = width < 1024 ? GRID_SIZE - 20 : GRID_SIZE;
    const subDivisions = Math.ceil(width / (gridSize + LINE_WIDTH));
    const numLines = subDivisions + (subDivisions % 2) - 1;
    for (let i = 0; i < numLines; i++) {
      const cssVars = {
        "--pulse-color": i < numLines / 2 ? "#2b99ff" : "#f060c0",
        "--delay": `-${i + Math.random() * 3}s`,
      };
      if (Math.abs(i - numLines / 2) <= 2) {
        cssVars["--pulse-color"] = "rgba(0,0,0,0)";
      }
      props.push({
        key: `vertical-${i}`,
        className: styles.pulse,
        style: {
          width: LINE_WIDTH,
          height: "100%",
          display: "block",
          position: "relative",
          overflow: "hidden",
          marginRight: gridSize - LINE_WIDTH,
          ...cssVars,
        },
      });
    }
    setVerticalLineProps(props);
  }, [width]);

  return (
    <div
      className={cn(
        "![perspective:1000px] sm:![perspective:1000px] md:![perspective:1000px] lg:![perspective:1000px]",
        styles.container
      )}
      ref={contentRef}
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
        <div className={styles.lines}>
          {verticalLineProps.map(({ key, ...props }) => (
            <span key={key} {...props} />
          ))}
        </div>
      </div>
    </div>
  );
}
