import { TheNumber } from "@/components/RemoteCacheCounterButRsc/TheNumber";
import {
  computeTimeSaved,
  remoteCacheTimeSavedQuery,
  REMOTE_CACHE_MINUTES_SAVED_URL,
} from "@/components/RemoteCacheCounterButRsc/data";
import { SwrProvider } from "@/components/RemoteCacheCounterButRsc/swr-provider";

export const NumberWrapper = async () => {
  const startingAnimationNumber = computeTimeSaved(
    await remoteCacheTimeSavedQuery(REMOTE_CACHE_MINUTES_SAVED_URL)
  );
  return (
    <SwrProvider startingNumber={startingAnimationNumber}>
      <TheNumber />
    </SwrProvider>
  );
};
