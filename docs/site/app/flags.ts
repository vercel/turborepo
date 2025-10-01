import { flag } from "flags/next";

export const dummyFlag = flag<boolean>({
  key: "dummy-flag",
  description: "Dummy flag to make sure the flags set-up works",
  defaultValue: true,
  decide() {
    return true;
  },
});
