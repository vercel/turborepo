import React from 'react'
import { useRouter } from 'next/router'
import Link from 'next/link'
import cn from 'classnames'

import useMounted from './utils/use-mounted'

export default function LocaleSwitch({ options, isRTL }) {
  const { locale, asPath } = useRouter()
  const mounted = useMounted()

  return (
    <details className="locale-switch relative">
      <summary
        className="text-current p-2 cursor-pointer outline-none"
        tabIndex="0"
      >
        <svg
          fill="none"
          viewBox="0 0 24 24"
          width="24"
          height="24"
          stroke="currentColor"
          aria-hidden="true"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 5h12M9 3v2m1.048 9.5A18.022 18.022 0 016.412 9m6.088 9h7M11 21l5-10 5 10M12.751 5C11.783 10.77 8.07 15.61 3 18.129"
          />
        </svg>
        <span className="sr-only">Languages</span>
      </summary>
      {mounted ? (
        <ul
          className={cn(
            'locale-dropdown absolute block bg-white dark:bg-dark border dark:border-gray-700 py-1 rounded shadow-lg',
            { 'right-0': !isRTL, 'left-0': isRTL }
          )}
        >
          {options.map(l => (
            <li key={l.locale}>
              <Link href={asPath} locale={l.locale}>
                <a
                  className={cn(
                    'block no-underline text-current py-2 px-4 hover:bg-gray-200 dark:hover:bg-gray-800 whitespace-nowrap',
                    {
                      'font-semibold': locale === l.locale,
                      'bg-gray-100 dark:bg-gray-900': locale === l.locale
                    }
                  )}
                >
                  {l.text}
                </a>
              </Link>
            </li>
          ))}
        </ul>
      ) : null}
    </details>
  )
}
