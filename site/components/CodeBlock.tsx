import rangeParser from 'parse-numeric-range'
import Highlight, {
  defaultProps,
  Language,
  PrismTheme,
} from 'prism-react-renderer'
import themeDark from 'prism-react-renderer/themes/nightOwl'
import themeLight from 'prism-react-renderer/themes/nightOwlLight'
import * as React from 'react'
import { IconClipboard } from './Icon/IconClipboard'
import { useClipboard } from './useClipboard'
import Prism from 'prism-react-renderer/prism'
import { useTheme } from 'next-themes'
;(typeof global !== 'undefined' ? global : (window as any)).Prism = Prism

require('prismjs/components/prism-docker')
/**
 * This is the code block component.
 *
 * When used with MDX, it receives words passed after the langauge declaration as
 * a prop called `metastring` as well as independent props.
 *
 */
function CodeBlock({
  children,
  metastring,
  ...props
}: {
  children: string
  codeString: string
  metastring: string
  className?: string
}) {
  const [didCopy, handleCopyToClipboard] = useClipboard(children.trim())
  const { theme } = useTheme()
  return (
    <Highlight
      {...defaultProps}
      Prism={Prism}
      code={children.trim()}
      language={
        (props.className?.replace('language-', '') as Language) ??
        ('jsx' as Language)
      }
      theme={theme === 'light' ? themeLight : (themeDark as PrismTheme)}
    >
      {({ className, style, tokens, getLineProps, getTokenProps }) => {
        const shouldHighlightLine = calculateLinesToHighlight(metastring)
        return (
          <div className="relative mb-6">
            <pre
              style={style}
              className={
                className +
                '  !bg-gray-50 dark:!bg-gray-900 dark:!bg-opacity-25 border border-gray-100 dark:border-gray-800 actually-word-break py-4 -mx-4 !px-0 whitespace-pre-wrap sm:rounded-lg !text-sm !leading-6 relative overflow-x-auto scrolling-touch'
              }
            >
              {tokens.map((line, i) => {
                const lineProps = getLineProps({ line, key: i })
                const newLinesProps = { ...lineProps }
                if (shouldHighlightLine(i)) {
                  newLinesProps.className = ` ${newLinesProps.className} bg-gray-100 dark:bg-gray-500 dark:bg-opacity-10  `
                }
                return (
                  <div
                    key={newLinesProps.key}
                    {...newLinesProps}
                    className={newLinesProps.className + ' px-4'}
                  >
                    {line.map((token, key) => (
                      <span key={key} {...getTokenProps({ token, key })} />
                    ))}
                  </div>
                )
              })}
            </pre>

            <button
              type="button"
              className="text-gray-600 bg-transparent rounded absolute top-0 text-xs font-medium right-0 font-sans inline-flex items-center px-2 leading-5 py-1 m-2  betterhover:hover:bg-gray-100 active:bg-gray-200 focus:shadow-outline outline-none transition duration-150 ease-out betterhover:hover:text-gray-900 focus:outline-none md:-mr-2"
              aria-label="Copy to clipboard"
              onClick={handleCopyToClipboard}
            >
              <IconClipboard
                className="h-3 w-3  mr-1 text-current"
                aria-hidden="true"
              />
              <span>{didCopy ? 'Copied!' : 'Copy'}</span>
            </button>
          </div>
        )
      }}
    </Highlight>
  )
}

CodeBlock.displayName = 'CodeBlock'

export default CodeBlock

/**
 * Returns fn to
 * @param metastring The text that comes after the language in a markdown block (requires a space)
 *
 * @example
 * ```js {10} live foo
 * ...
 * ```
 *
 * -> The metastring is `{10} live foo`
 */
function calculateLinesToHighlight(metastring: string) {
  const RE = /{([\d,-]+)}/
  const meta = RE.exec(metastring)
  if (meta === null) {
    return () => false
  }
  const strlineNumbers = meta[1]
  const lineNumbers = rangeParser(strlineNumbers)
  return (index: number) => lineNumbers.includes(index + 1)
}
