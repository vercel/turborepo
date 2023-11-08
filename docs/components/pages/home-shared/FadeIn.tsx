import { motion, useInView } from "framer-motion";
import { useRef } from "react";

export function FadeIn({
  children,
  className,
  noVertical,
  delay,
  viewTriggerOffset,
}: {
  children: React.ReactNode;
  className?: string;
  noVertical?: boolean;
  delay?: number;
  viewTriggerOffset?: boolean;
}) {
  const ref = useRef(null);
  const inView = useInView(ref, {
    once: true,
    margin: viewTriggerOffset ? "-128px" : "0px",
  });

  const fadeUpVariants = {
    initial: {
      opacity: 0,
      y: noVertical ? 0 : 24,
    },
    animate: {
      opacity: 1,
      y: 0,
    },
  };

  return (
    <motion.div
      animate={inView ? "animate" : "initial"}
      className={className}
      initial={false}
      ref={ref}
      transition={{
        duration: 1,
        delay: delay || 0,
        ease: [0.21, 0.47, 0.32, 0.98],
      }}
      variants={fadeUpVariants}
    >
      {children}
    </motion.div>
  );
}
