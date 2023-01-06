import { AnimatePresence, motion } from "framer-motion";
import { useTurboSite } from "../SiteSwitcher";
import PackSymbol from "./PackSymbol";
import RepoSymbol from "./RepoSymbol";
import TurboWordmark from "./TurboWordmark";

const VERTICAL_OFFSET = 23;
const ANIMATION_SPEED = 0.4;

const animatedSymbol = (children: JSX.Element, key: string) =>
  <motion.div
    key={key}
    style={{ position: "absolute", top: 0, left: 0 }}
    initial={{ opacity: 0, y: VERTICAL_OFFSET}}
    animate={{ opacity: 1, y: 0 }}
    exit={{ opacity: 0, y: -VERTICAL_OFFSET }}
    transition={{ duration: ANIMATION_SPEED, ease: "easeOut" }}
  >
    {children}
  </motion.div>

const AnimatedLogo = () => {
  const symbol = useTurboSite();

  return (
      <div className="flex items-center gap-2">
        <div className="relative w-[25px] h-[21px]">
          <AnimatePresence>
            {symbol === "pack"
              ? animatedSymbol(<PackSymbol />, 'pack')
              : animatedSymbol(<RepoSymbol />, 'repo')}
          </AnimatePresence>
        </div>
        <TurboWordmark />
      </div>
  )
};

export default AnimatedLogo;
