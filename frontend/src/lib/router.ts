import { useEffect, useState } from 'react'

export type Route =
  | { name: 'home' }
  | { name: 'processing'; job: string }
  | { name: 'song'; id: string; tab: 'overview' | 'sheet' | 'score' | 'adjust' }

function parse(hash: string): Route {
  const parts = hash.replace(/^#\/?/, '').split('/').filter(Boolean)
  if (parts[0] === 'new' && parts[1]) return { name: 'processing', job: parts[1] }
  if (parts[0] === 'song' && parts[1]) {
    const tab = parts[2] === 'sheet' || parts[2] === 'score' || parts[2] === 'adjust' ? parts[2] : 'overview'
    return { name: 'song', id: parts[1], tab }
  }
  return { name: 'home' }
}

export const go = (path: string) => { window.location.hash = path }

export function useRoute(): Route {
  const [route, setRoute] = useState<Route>(() => parse(window.location.hash))
  useEffect(() => {
    const on = () => setRoute(parse(window.location.hash))
    window.addEventListener('hashchange', on)
    return () => window.removeEventListener('hashchange', on)
  }, [])
  return route
}
