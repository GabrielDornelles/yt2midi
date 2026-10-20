import { useCallback, useEffect, useState } from 'react'

type Theme = 'light' | 'dark'
const system = (): Theme => (window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(() => (document.documentElement.dataset.theme as Theme) || system())
  useEffect(() => { document.documentElement.dataset.theme = theme }, [theme])
  const toggle = useCallback(() => {
    setTheme((t) => {
      const n = t === 'dark' ? 'light' : 'dark'
      try { localStorage.setItem('theme', n) } catch { /* private mode */ }
      return n
    })
  }, [])
  return { theme, toggle }
}
