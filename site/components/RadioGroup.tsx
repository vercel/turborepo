import * as React from 'react'
import { VisuallyHidden } from '@react-aria/visually-hidden'
import { useFocusRing } from '@react-aria/focus'
import { RadioGroupProps } from '@react-types/radio'
import { useRadioGroupState, RadioGroupState } from '@react-stately/radio'
import { useRadioGroup, useRadio, RadioAriaProps } from '@react-aria/radio'
// RadioGroup is the same as in the previous example
let RadioContext = React.createContext<RadioGroupState>({} as RadioGroupState)

export function RadioGroup(
  props: RadioGroupProps & { children: React.ReactNode }
) {
  let { children, label } = props
  let state = useRadioGroupState(props)
  let { radioGroupProps, labelProps } = useRadioGroup(props, state)

  return (
    <div {...radioGroupProps}>
      <span {...labelProps}>{label}</span>
      <div className="space-y-2">
        <RadioContext.Provider value={state}>{children}</RadioContext.Provider>
      </div>
    </div>
  )
}

export function Radio(props: RadioAriaProps) {
  let { children } = props
  let state = React.useContext(RadioContext)
  let ref = React.useRef<HTMLElement>(null)
  let { inputProps } = useRadio(props, state, ref)
  let { isFocusVisible, focusProps } = useFocusRing()

  let isSelected = state.selectedValue === props.value
  let strokeWidth = isSelected ? 6 : 2

  return (
    <label className="flex items-center py-3 px-5 bg-gray-100 bg-opacity-5 text-white rounded-md  leading-6 font-medium  betterhover:hover:text-gray-200 focus:outline-none  betterhover:hover:bg-opacity-25 focus:border-blue-300 focus:shadow-outline-blue active:bg-gray-50 active:text-gray-800 transition duration-150 ease-in-out">
      <VisuallyHidden>
        <input {...inputProps} {...focusProps} />
      </VisuallyHidden>
      <svg
        width={24}
        height={24}
        aria-hidden="true"
        style={{ marginRight: 4 }}
        className={isSelected ? 'text-blue-500' : 'text-gray-600'}
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
  )
}
