import * as React from 'react'
import { SliderState, useSliderState } from '@react-stately/slider'
import { useSlider, useSliderThumb } from '@react-aria/slider'
import { useFocusRing } from '@react-aria/focus'
import { VisuallyHidden } from '@react-aria/visually-hidden'
import { mergeProps } from '@react-aria/utils'
import { useNumberFormatter } from '@react-aria/i18n'
import { AriaSliderProps, AriaSliderThumbProps } from '@react-types/slider'
import { AriaNumberFieldProps } from '@react-types/numberfield'

export function Slider(
  props: AriaSliderProps & Pick<AriaNumberFieldProps, 'formatOptions'>
) {
  let trackRef = React.useRef(null)
  let numberFormatter = useNumberFormatter(props.formatOptions)
  let state = useSliderState({ ...props, numberFormatter })
  let { groupProps, trackProps, labelProps, outputProps } = useSlider(
    props,
    state,
    trackRef
  )

  return (
    <div
      {...groupProps}
      style={{
        position: 'relative',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',

        touchAction: 'none',
      }}
    >
      {/* Create a flex container for the label and output element. */}
      <div style={{ display: 'flex', alignSelf: 'stretch' }}>
        {props.label && (
          <label className="text-blue-400 text-xl font-medium" {...labelProps}>
            {props.label}
          </label>
        )}
        <output
          {...outputProps}
          className="text-blue-400 text-xl font-bold pl-6"
          style={{ flex: '1 0 auto', textAlign: 'end' }}
        >
          {state.getThumbValueLabel(0)}
        </output>
      </div>
      {/* The track element holds the visible track line and the thumb. */}
      <div
        {...trackProps}
        ref={trackRef}
        className="mt-2"
        style={{
          position: 'relative',
          height: 30,
          width: ' 100%',
        }}
      >
        <div
          style={{
            position: 'absolute',

            height: 2,
            top: 13,
            width: '100%',
          }}
          className="bg-blue-400 bg-opacity-50"
        />
        <Thumb index={0} state={state} trackRef={trackRef} />
      </div>
    </div>
  )
}

function Thumb(
  props: AriaSliderThumbProps & { state: SliderState; trackRef: any }
) {
  let { state, trackRef, index } = props
  let inputRef = React.useRef(null)
  let { thumbProps, inputProps } = useSliderThumb(
    {
      index,
      trackRef,
      inputRef,
    },
    state
  )

  let { focusProps, isFocusVisible } = useFocusRing()
  return (
    <div
      style={{
        position: 'absolute',
        top: 4,
        transform: 'translateX(-50%)',
        left: `${state.getThumbPercent(index) * 100}%`,
      }}
    >
      <div
        {...thumbProps}
        style={{
          width: 20,
          height: 20,
          borderRadius: '50%',
        }}
        className={
          isFocusVisible
            ? 'bg-blue-200 border border-blue-400 shadow'
            : state.isThumbDragging(index)
            ? 'bg-blue-500 border border-blue-400 shadow'
            : 'bg-blue-400 border border-blue-400 shadow'
        }
      >
        <VisuallyHidden>
          <input ref={inputRef} {...mergeProps(inputProps, focusProps)} />
        </VisuallyHidden>
      </div>
    </div>
  )
}
