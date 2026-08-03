/**
 * Icon sprite, ported from prototype/index.html. Rendered once at the app root;
 * every <Icon> is a <use href="#i-…"> against it.
 */
export function IconSprite() {
  return (
    <svg style={{ display: 'none' }} xmlns="http://www.w3.org/2000/svg" aria-hidden>
      <defs>
        <g id="i-library">
          <path d="M4 5h5v14H4zM11 5h4v14h-4zM17 6l3 12" />
        </g>
        <g id="i-forge">
          <path d="M12 3l2.2 5.2 5.8.5-4.4 3.8 1.3 5.6L12 15.2 7.1 18.1l1.3-5.6L4 8.7l5.8-.5z" />
        </g>
        <g id="i-assets">
          <rect x="3" y="4" width="18" height="16" rx="2" />
          <circle cx="8.5" cy="9.5" r="1.6" />
          <path d="M21 16l-5-5-9 9" />
        </g>
        <g id="i-settings">
          <circle cx="12" cy="12" r="3" />
          <path d="M12 2v3M12 19v3M4.2 4.2l2.1 2.1M17.7 17.7l2.1 2.1M2 12h3M19 12h3M4.2 19.8l2.1-2.1M17.7 6.3l2.1-2.1" />
        </g>
        <g id="i-search">
          <circle cx="11" cy="11" r="7" />
          <path d="M20 20l-3.5-3.5" />
        </g>
        <g id="i-style">
          <path d="M12 3l9 5-9 5-9-5z" />
          <path d="M3 13l9 5 9-5" />
        </g>
        <g id="i-world">
          <circle cx="12" cy="12" r="9" />
          <path d="M3 12h18M12 3c2.6 3 2.6 15 0 18M12 3c-2.6 3-2.6 15 0 18" />
        </g>
        <g id="i-species">
          <path d="M7 3c0 6 10 6 10 12M17 3c0 6-10 6-10 12M7 21c0-2 10-2 10 0" />
          <path d="M8.5 8h7M8.5 13h7" />
        </g>
        <g id="i-culture">
          <circle cx="9" cy="8" r="3" />
          <path d="M3 20c0-3.3 2.7-6 6-6s6 2.7 6 6" />
          <path d="M16 5.5a3 3 0 010 5.6M18 20c0-2.6-1-4.5-2.5-5.6" />
        </g>
        <g id="i-place">
          <path d="M12 21s7-6.3 7-11a7 7 0 10-14 0c0 4.7 7 11 7 11z" />
          <circle cx="12" cy="10" r="2.5" />
        </g>
        <g id="i-character">
          <circle cx="12" cy="7.5" r="3.8" />
          <path d="M4.5 21c0-4.1 3.4-7.5 7.5-7.5s7.5 3.4 7.5 7.5" />
        </g>
        <g id="i-creature">
          <path d="M4 15c0-3.9 3.6-7 8-7s8 3.1 8 7" />
          <path d="M6 8.5L4.5 4l4 2.2M18 8.5L19.5 4l-4 2.2" />
          <path d="M9.5 13.5h.01M14.5 13.5h.01M4 15v4M20 15v4" />
        </g>
        <g id="i-prop">
          <path d="M12 2.8l8 4.4v9.6l-8 4.4-8-4.4V7.2z" />
          <path d="M4 7.2l8 4.4 8-4.4M12 11.6V21" />
        </g>
        <g id="i-env">
          <path d="M3 18l5.5-7 4 5 3-3.6L21 18z" />
          <circle cx="8" cy="6.5" r="2.2" />
          <path d="M3 21h18" />
        </g>
        <g id="i-vehicle">
          <path d="M3 16v-3.2l2-4.3A2 2 0 016.8 7h10.4a2 2 0 011.8 1.5l2 4.3V16" />
          <path d="M3 16h18M5.5 16v2M18.5 16v2" />
          <circle cx="7.5" cy="16" r="1.6" />
          <circle cx="16.5" cy="16" r="1.6" />
        </g>
        <g id="i-cube">
          <path d="M12 2.8l8 4.4v9.6l-8 4.4-8-4.4V7.2z" />
          <path d="M4 7.2l8 4.4 8-4.4M12 11.6V21" />
        </g>
        <g id="i-spark">
          <path d="M12 3l1.6 4.4L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.6z" />
          <path d="M18.5 14.5l.7 1.9 1.9.7-1.9.7-.7 1.9-.7-1.9-1.9-.7 1.9-.7z" />
        </g>
        <g id="i-plus">
          <path d="M12 5v14M5 12h14" />
        </g>
        <g id="i-minus">
          <path d="M5 12h14" />
        </g>
        <g id="i-chev">
          <path d="M9 6l6 6-6 6" />
        </g>
        <g id="i-dots">
          <circle cx="5" cy="12" r="1.4" />
          <circle cx="12" cy="12" r="1.4" />
          <circle cx="19" cy="12" r="1.4" />
        </g>
        <g id="i-copy">
          <rect x="8.5" y="8.5" width="11.5" height="11.5" rx="2" />
          <path d="M15.5 5.5A1.5 1.5 0 0014 4H5.5A1.5 1.5 0 004 5.5V14a1.5 1.5 0 001.5 1.5" />
        </g>
        <g id="i-trash">
          <path d="M4 7h16M9.5 7V4.8A.8.8 0 0110.3 4h3.4a.8.8 0 01.8.8V7" />
          <path d="M6.5 7l.9 12.2a1.8 1.8 0 001.8 1.8h5.6a1.8 1.8 0 001.8-1.8L17.5 7" />
        </g>
        <g id="i-pin">
          <path d="M9 3h6l-1 6 4 3v2H6v-2l4-3z" />
          <path d="M12 14v7" />
        </g>
        <g id="i-share">
          <path d="M4 13v5a2 2 0 002 2h12a2 2 0 002-2v-5" />
          <path d="M12 3v12M8 7l4-4 4 4" />
        </g>
        <g id="i-x">
          <path d="M6 6l12 12M18 6L6 18" />
        </g>
        <g id="i-lock">
          <rect x="5" y="10.5" width="14" height="10" rx="2" />
          <path d="M8.5 10.5V7.5a3.5 3.5 0 017 0v3" />
        </g>
        <g id="i-folder">
          <path d="M3 7a2 2 0 012-2h4l2 2.4h8a2 2 0 012 2V18a2 2 0 01-2 2H5a2 2 0 01-2-2z" />
        </g>
        <g id="i-refresh">
          <path d="M20 12a8 8 0 11-2.6-5.9M20 4v4.5h-4.5" />
        </g>
        <g id="i-check">
          <path d="M5 12.5l4.5 4.5L19 7.5" />
        </g>
        <g id="i-clock">
          <circle cx="12" cy="12" r="8.5" />
          <path d="M12 7.5V12l3 2" />
        </g>
        <g id="i-layers">
          <path d="M12 3l9 5-9 5-9-5z" />
          <path d="M3 12l9 5 9-5M3 16l9 5 9-5" />
        </g>
        <g id="i-link">
          <path d="M10 13.5a4 4 0 006 .5l2.5-2.5a4 4 0 00-5.7-5.7L11.4 7" />
          <path d="M14 10.5a4 4 0 00-6-.5L5.5 12.5a4 4 0 005.7 5.7L12.6 17" />
        </g>
        <g id="i-image">
          <rect x="3" y="4" width="18" height="16" rx="2" />
          <circle cx="8.5" cy="9.5" r="1.6" />
          <path d="M21 16l-5-5-9 9" />
        </g>
        <g id="i-win-min">
          <path d="M5 12h14" />
        </g>
        <g id="i-win-max">
          <rect x="5" y="5" width="14" height="14" rx="1.5" />
        </g>
        <g id="i-win-restore">
          <rect x="4" y="8" width="12" height="12" rx="1.5" />
          <path d="M8 8V5.5A1.5 1.5 0 019.5 4h9A1.5 1.5 0 0120 5.5v9A1.5 1.5 0 0118.5 16H16" />
        </g>
      </defs>
    </svg>
  )
}
