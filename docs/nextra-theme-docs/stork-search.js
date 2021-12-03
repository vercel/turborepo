import React, {
  useMemo,
  useCallback,
  useRef,
  useState,
  useEffect,
  Fragment
} from 'react'
import Router, { useRouter } from 'next/router'
import cn from 'classnames'
import Link from 'next/link'
import GraphemeSplitter from 'grapheme-splitter'

const splitter = new GraphemeSplitter()

const TextWithHighlights = React.memo(({ content, ranges }) => {
  const splittedText = content ? splitter.splitGraphemes(content) : []
  const res = []

  let id = 0,
    index = 0
  for (const range of ranges) {
    res.push(
      <Fragment key={id++}>
        {splittedText.splice(0, range.beginning - index).join('')}
      </Fragment>
    )
    res.push(
      <span className="highlight" key={id++}>
        {splittedText.splice(0, range.end - range.beginning).join('')}
      </span>
    )
    index = range.end
  }
  res.push(<Fragment key={id++}>{splittedText.join('')}</Fragment>)

  return res
})

const Item = ({ title, active, href, onMouseOver, excerpt }) => {
  return (
    <Link href={href}>
      <a className="block no-underline" onMouseOver={onMouseOver}>
        <li className={cn('py-2 px-4', { active })}>
          <span className="font-semibold">{title}</span>
          {excerpt ? (
            <div className="text-gray-600">
              <TextWithHighlights
                content={excerpt.text}
                ranges={excerpt.highlight_ranges}
              />
            </div>
          ) : null}
        </li>
      </a>
    </Link>
  )
}

// This can be global for better caching.
const stork = {}

export default function Search() {
  const router = useRouter()
  const [show, setShow] = useState(false)
  const [search, setSearch] = useState('')
  const [active, setActive] = useState(0)
  const setStork = useState({})[1]
  const input = useRef(null)

  const results = useMemo(() => {
    if (!search) return []

    const localeCode = Router.locale || 'default'
    if (!stork[localeCode]) return []

    try {
      const json = stork[localeCode].wasm_search(`index-${localeCode}`, search)
      const obj = JSON.parse(json)

      if (!obj.results) return []
      return obj.results.slice(0, 20).map(result => {
        return {
          title: result.entry.title,
          route: result.entry.url,
          excerpt: result.excerpts[0]
        }
      })
    } catch (err) {
      console.error(err)
      return []
    }
  }, [search])

  const handleKeyDown = useCallback(
    e => {
      switch (e.key) {
        case 'ArrowDown': {
          e.preventDefault()
          if (active + 1 < results.length) {
            setActive(active + 1)
            const activeElement = document.querySelector(
              `.nextra-stork ul > :nth-child(${active + 2})`
            )
            if (activeElement && activeElement.scrollIntoViewIfNeeded) {
              activeElement.scrollIntoViewIfNeeded()
            }
          }
          break
        }
        case 'ArrowUp': {
          e.preventDefault()
          if (active - 1 >= 0) {
            setActive(active - 1)
            const activeElement = document.querySelector(
              `.nextra-stork ul > :nth-child(${active})`
            )
            if (activeElement && activeElement.scrollIntoViewIfNeeded) {
              activeElement.scrollIntoViewIfNeeded()
            }
          }
          break
        }
        case 'Enter': {
          router.push(results[active].route)
          break
        }
      }
    },
    [active, results, router]
  )

  const load = async () => {
    const localeCode = Router.locale || 'default'
    if (!stork[localeCode]) {
      stork[localeCode] = await import('./wasm-loader')
      setStork({})

      const init = stork[localeCode].init('/stork.wasm')
      const res = await fetch(`/index-${localeCode}.st`)
      const buf = await res.arrayBuffer()
      await init
      stork[localeCode].wasm_register_index(
        `index-${localeCode}`,
        new Uint8Array(buf)
      )
    }
  }

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
    <div className="relative w-full nextra-search nextra-stork md:w-64">
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
          onFocus={() => {
            load()
            setShow(true)
          }}
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
        <ul className="absolute z-20 p-0 m-0 mt-1 divide-y top-full">
          {results.map((res, i) => {
            return (
              <Item
                key={`search-item-${i}`}
                title={res.title}
                href={res.route}
                excerpt={res.excerpt}
                active={i === active}
                onMouseOver={() => setActive(i)}
              />
            )
          })}
        </ul>
      )}
    </div>
  )
}
