import React, { useMemo, useCallback, useRef, useState, useEffect } from 'react'
import matchSorter from 'match-sorter'
import cn from 'classnames'
import { useRouter } from 'next/router'
import Link from 'next/link'

const Item = ({ title, active, href, onMouseOver, search }) => {
  const highlight = title.toLowerCase().indexOf(search.toLowerCase())

  return (
    <Link href={href}>
      <a className="block no-underline" onMouseOver={onMouseOver}>
        <li className={cn('p-2', { active })}>
          {title.substring(0, highlight)}
          <span className="highlight">
            {title.substring(highlight, highlight + search.length)}
          </span>
          {title.substring(highlight + search.length)}
        </li>
      </a>
    </Link>
  )
}

const UP = true
const DOWN = false

const Search = ({ directories = [] }) => {
  const router = useRouter()
  const [show, setShow] = useState(false)
  const [search, setSearch] = useState('')
  const [active, setActive] = useState(0)
  const input = useRef(null)

  const results = useMemo(() => {
    if (!search) return []

    // Will need to scrape all the headers from each page and search through them here
    // (similar to what we already do to render the hash links in sidebar)
    // We could also try to search the entire string text from each page
    return matchSorter(directories, search, { keys: ['title'] })
  }, [search])

  const moveActiveItem = up => {
    const position = active + (up ? -1 : 1)
    const { length } = results

    // Modulo instead of remainder,
    // see https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Operators/Remainder
    const next = (position + length) % length
    setActive(next)
  }

  const handleKeyDown = useCallback(
    e => {
      const { key, ctrlKey } = e

      if ((ctrlKey && key === 'n') || key === 'ArrowDown') {
        e.preventDefault()
        moveActiveItem(DOWN)
      }

      if ((ctrlKey && key === 'p') || key === 'ArrowUp') {
        e.preventDefault()
        moveActiveItem(UP)
      }

      if (key === 'Enter') {
        router.push(results[active].route)
      }
    },
    [active, results, router]
  )

  useEffect(() => {
    setActive(0)
  }, [search])

  useEffect(() => {
    const inputs = ['input', 'select', 'button', 'textarea']

    const down = e => {
      if (
        document.activeElement &&
        inputs.indexOf(document.activeElement.tagName.toLowerCase()) === -1
      ) {
        if (e.key === '/') {
          e.preventDefault()
          input.current.focus()
        } else if (e.key === 'Escape') {
          setShow(false)
        }
      }
    }

    window.addEventListener('keydown', down)
    return () => window.removeEventListener('keydown', down)
  }, [])

  const renderList = show && results.length > 0

  return (
    <div className="relative w-full nextra-search md:w-64">
      {renderList && (
        <div className="z-10 search-overlay" onClick={() => setShow(false)} />
      )}

      <div className="relative flex items-center">
        <input
          onChange={e => {
            setSearch(e.target.value)
            setShow(true)
          }}
          className="block w-full px-3 py-2 leading-tight border rounded appearance-none focus:outline-none focus:ring"
          type="search"
          placeholder="Search documentation..."
          onKeyDown={handleKeyDown}
          onFocus={() => setShow(true)}
          onBlur={() => setShow(false)}
          ref={input}
          spellCheck={false}
        />
        {show ? null : (
          <div className="hidden sm:flex absolute inset-y-0 right-0 py-1.5 pr-1.5">
            <kbd className="inline-flex items-center px-2 font-sans text-sm font-medium text-gray-400 dark:text-gray-800 dark:border-gray-800 border rounded">
              /
            </kbd>
          </div>
        )}
      </div>
      {renderList && (
        <ul className="absolute left-0 z-20 w-full p-0 m-0 mt-1 list-none border divide-y rounded shadow-md md:right-0 top-100 md:w-auto">
          {results.map((res, i) => {
            return (
              <Item
                key={`search-item-${i}`}
                title={res.title}
                href={res.route}
                active={i === active}
                search={search}
                onMouseOver={() => setActive(i)}
              />
            )
          })}
        </ul>
      )}
    </div>
  )
}

export default Search
