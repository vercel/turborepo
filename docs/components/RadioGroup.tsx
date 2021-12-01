import { useFocusRing } from "@react-aria/focus";
import { useRadio, useRadioGroup } from "@react-aria/radio";
import { RadioGroupState, useRadioGroupState } from "@react-stately/radio";
import { RadioGroupProps } from "@react-types/radio";
import * as React from "react";

let RadioContext = React.createContext<RadioGroupState>({} as RadioGroupState);

export function RadioGroup(
  props: RadioGroupProps & { children: React.ReactNode }
) {
  let { children, label } = props;
  let state = useRadioGroupState(props);
  let { radioGroupProps, labelProps } = useRadioGroup(props, state);

  return (
    <div {...radioGroupProps}>
      <span {...labelProps}>{label}</span>
      <div className="space-y-2">
        <RadioContext.Provider value={state}>{children}</RadioContext.Provider>
      </div>
    </div>
  );
}

type Arguments<F extends Function> = F extends (...args: infer A) => any
  ? A
  : never;
type RadioProps = Arguments<typeof useRadio>[0];

export function Radio(props: RadioProps) {
  let { children } = props;
  let state = React.useContext(RadioContext);
  let ref = React.useRef<HTMLElement>(null);
  let { inputProps } = useRadio(props, state, ref);
  let { isFocusVisible, focusProps } = useFocusRing();

  let isSelected = state.selectedValue === props.value;
  let strokeWidth = isSelected ? 6 : 2;

  return (
    <label className="flex items-center px-5 py-3 font-medium leading-6 text-gray-900 transition duration-150 ease-in-out bg-gray-100 rounded-md dark:bg-opacity-5 dark:text-white betterhover:dark:hover:text-gray-200 focus:outline-none betterhover:dark:hover:bg-opacity-25 betterhover:hover:bg-gray-200 focus:border-blue-300 focus:shadow-outline-blue active:bg-gray-50 active:text-gray-800">
      <span className="sr-only">
        <input {...inputProps} {...focusProps} />
      </span>
      <svg
        width={24}
        height={24}
        aria-hidden="true"
        style={{ marginRight: 4 }}
        className={isSelected ? "text-blue-500" : "text-gray-600"}
      >
        <circle
          cx={12}
          cy={12}
          r={8 - strokeWidth / 2}
          fill="none"
          stroke="currentColor"
          strokeWidth={strokeWidth}
        />
        {isFocusVisible && (
          <circle
            cx={12}
            cy={12}
            r={11}
            fill="none"
            stroke="currentColor"
            strokeWidth={2}
          />
        )}
      </svg>
      {children}
    </label>
  );
}
