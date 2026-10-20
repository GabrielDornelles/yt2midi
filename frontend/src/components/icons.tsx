import type { ReactNode } from 'react'

const Svg = ({ children, size = 18 }: { children: ReactNode; size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    {children}
  </svg>
)

export const Download = () => <Svg><path d="M12 3v12m0 0l-4-4m4 4l4-4M4 20h16" /></Svg>
export const Play = () => <Svg><path d="M7 4.5v15l12-7.5z" fill="currentColor" /></Svg>
export const Pause = () => <Svg><path d="M7 5h3.5v14H7zM13.5 5H17v14h-3.5z" fill="currentColor" /></Svg>
export const Swap = () => <Svg><path d="M7 4L3 8l4 4M3 8h14M17 20l4-4-4-4m4 4H7" /></Svg>
export const Check = () => <Svg size={15}><path d="M5 12.5l4.5 4.5L19 7.5" /></Svg>
export const Gear = () => <Svg><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 00.3 1.8l.1.1a2 2 0 11-2.8 2.8l-.1-.1a1.7 1.7 0 00-1.8-.3 1.7 1.7 0 00-1 1.5V21a2 2 0 11-4 0v-.1a1.7 1.7 0 00-1.1-1.5 1.7 1.7 0 00-1.8.3l-.1.1a2 2 0 11-2.8-2.8l.1-.1a1.7 1.7 0 00.3-1.8 1.7 1.7 0 00-1.5-1H3a2 2 0 110-4h.1a1.7 1.7 0 001.5-1.1 1.7 1.7 0 00-.3-1.8l-.1-.1a2 2 0 112.8-2.8l.1.1a1.7 1.7 0 001.8.3H9a1.7 1.7 0 001-1.5V3a2 2 0 114 0v.1a1.7 1.7 0 001 1.5 1.7 1.7 0 001.8-.3l.1-.1a2 2 0 112.8 2.8l-.1.1a1.7 1.7 0 00-.3 1.8V9a1.7 1.7 0 001.5 1H21a2 2 0 110 4h-.1a1.7 1.7 0 00-1.5 1z" /></Svg>
export const Trash = () => <Svg size={16}><path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" /></Svg>
export const Alert = () => <Svg size={20}><path d="M12 3l10 18H2zM12 10v5M12 18h.01" /></Svg>
export const ChevL = () => <Svg><path d="M15 5l-7 7 7 7" /></Svg>
export const ChevR = () => <Svg><path d="M9 5l7 7-7 7" /></Svg>
export const Sun = () => <Svg><circle cx="12" cy="12" r="4" /><path d="M12 2v2m0 16v2M4.9 4.9l1.4 1.4m11.4 11.4l1.4 1.4M2 12h2m16 0h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" /></Svg>
export const Moon = () => <Svg><path d="M21 13.5A9 9 0 1110.5 3 7 7 0 0021 13.5z" /></Svg>
export const Refresh = () => <Svg><path d="M20 11a8 8 0 00-14.5-4M4 4v4h4M4 13a8 8 0 0014.5 4M20 20v-4h-4" /></Svg>
export const Wand = () => <Svg><path d="M4 20L16 8m-2-4l1 2 2 1-2 1-1 2-1-2-2-1 2-1zM19 13l.7 1.3L21 15l-1.3.7L19 17l-.7-1.3L17 15l1.3-.7z" /></Svg>

export const Logo = () => (
  <svg width="30" height="30" viewBox="0 0 32 32" aria-hidden="true">
    <rect width="32" height="32" rx="8" fill="var(--accent)" />
    <path d="M8 7h4v12H8zM14 7h4v12h-4zM20 7h4v12h-4z" fill="#fffdf9" />
    <path d="M10 7h2v9h-2zM16 7h2v9h-2zM22 7h2v9h-2z" fill="#2d2620" />
  </svg>
)
