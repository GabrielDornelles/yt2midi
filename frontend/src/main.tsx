import { startPerfProbe } from './lib/perf'
import { installSmoothScroll } from './lib/smoothScroll'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { App } from './App'
import './app.css'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)

installSmoothScroll()
void startPerfProbe()
