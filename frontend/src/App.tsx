import { useEffect, useState } from 'react'
import { go, useRoute } from './lib/router'
import { useTheme } from './lib/theme'
import { Home } from './components/Home'
import { Processing } from './components/Processing'
import { SongPage } from './components/SongPage'
import { Logo, Moon, Sun } from './components/icons'

/** The desktop app saves downloads to the Downloads folder and tells the page; say where the file went. */
function SavedToast() {
  const [msg, setMsg] = useState<string | null>(null)
  useEffect(() => {
    let t = 0
    const on = (e: Event) => {
      const d = (e as CustomEvent<{ ok: boolean; path?: string }>).detail
      setMsg(d.ok && d.path ? `Saved to ${d.path}` : 'The file could not be saved')
      window.clearTimeout(t)
      t = window.setTimeout(() => setMsg(null), 7000)
    }
    window.addEventListener('yt2midi:saved', on)
    return () => { window.removeEventListener('yt2midi:saved', on); window.clearTimeout(t) }
  }, [])
  return msg ? <div className="toast" role="status">{msg}</div> : null
}

export function App() {
  const route = useRoute()
  const { theme, toggle } = useTheme()
  return (
    <div className="shell">
      <header className="topbar">
        <a className="brand" href="#/" onClick={(e) => { e.preventDefault(); go('/') }}>
          <Logo /> yt2midi
        </a>
        <span className="spacer" />
        <button className="btn ghost icon" onClick={toggle} aria-label={theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'}>
          {theme === 'dark' ? <Sun /> : <Moon />}
        </button>
      </header>
      <main className="page">
        {route.name === 'home' && <Home />}
        {route.name === 'processing' && <Processing job={route.job} />}
        {route.name === 'song' && <SongPage key={route.id} id={route.id} tab={route.tab} />}
      </main>
      <SavedToast />
    </div>
  )
}
