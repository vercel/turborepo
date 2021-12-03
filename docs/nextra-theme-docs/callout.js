import React from 'react'

const themes = {
  default:
    'bg-orange-100 text-orange-800 dark:text-orange-300 dark:bg-orange-200 dark:bg-opacity-10',
  error:
    'bg-red-200 text-red-900 dark:text-red-200 dark:bg-red-600 dark:bg-opacity-30',
  warning:
    'bg-yellow-200 text-yellow-900 dark:text-yellow-200 dark:bg-yellow-700 dark:bg-opacity-30'
}

export default ({ children, type = 'default', emoji = '💡' }) => {
  return (
    <div className={`${themes[type]} flex rounded-lg callout mt-6`}>
      <div
        className="pl-3 pr-2 py-2 select-none text-xl"
        style={{
          fontFamily: '"Apple Color Emoji", "Segoe UI Emoji", "Segoe UI Symbol"'
        }}
      >
        {emoji}
      </div>
      <div className="pr-4 py-2">{children}</div>
    </div>
  )
}
