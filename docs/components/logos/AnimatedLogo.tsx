import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { useTurboSite } from "../SiteSwitcher";
import PackSymbol from "./PackSymbol";
import RepoSymbol from "./RepoSymbol";
import TurboWordmark from "./TurboWordmark";
import { useEffect } from "react";

import styles from "./AnimatedLogo.module.css";

const SYMBOL_SIZE = 24;
const ANIMATION_DURATION = 0.4;

const AnimatedLogo = () => {
  const symbol = useTurboSite();

  const [hasLoadedOnce, setHasLoadedOnce] = useState(false);

  useEffect(() => setHasLoadedOnce(true), []);

  const animatedSymbol = (children: JSX.Element, key: string) =>
    <motion.div
      key={key}
      style={{ position: "absolute", top: 0, left: 0 }}
      initial={hasLoadedOnce
        ? { opacity: 0, y: SYMBOL_SIZE}
        : false}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -SYMBOL_SIZE }}
      transition={{ duration: ANIMATION_DURATION, ease: "easeOut" }}
    >
      {children}
    </motion.div>

  return (
      <div className="flex items-center gap-2">
        <div
          className="relative"
          style={{ width: SYMBOL_SIZE, height: SYMBOL_SIZE }}
        >
          <AnimatePresence>
            {symbol === "pack"
              ? animatedSymbol(<PackSymbol />, 'pack')
              : animatedSymbol(<RepoSymbol />, 'repo')}
          </AnimatePresence>
        </div>
        <TurboWordmark className={styles.desktopLogo} />
      </div>
  )
};

export default AnimatedLogo;
