// Inline stroke icons (24×24 grid, currentColor). No icon dependency: the path data is vendored from
// lucide-static 1.70.0 (ISC) and @tabler/icons 3.51.0 (MIT), with the line weight set to 1.75 and each
// icon joined into one path. Licences: apps/web/public/licenses/. Always decorative (aria-hidden):
// every place that shows one also has visible text or an aria-label.
const PATHS = {
  // lucide: info
  info: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0zM12 16v-4M12 8h.01",
  // lucide: layout-dashboard
  overview:
    "M4 3h5a1 1 0 0 1 1 1v7a1 1 0 0 1 -1 1h-5a1 1 0 0 1 -1 -1v-7a1 1 0 0 1 1 -1zM15 3h5a1 1 0 0 1 1 1v3a1 1 0 0 1 -1 1h-5a1 1 0 0 1 -1 -1v-3a1 1 0 0 1 1 -1zM15 12h5a1 1 0 0 1 1 1v7a1 1 0 0 1 -1 1h-5a1 1 0 0 1 -1 -1v-7a1 1 0 0 1 1 -1zM4 16h5a1 1 0 0 1 1 1v3a1 1 0 0 1 -1 1h-5a1 1 0 0 1 -1 -1v-3a1 1 0 0 1 1 -1z",
  // tabler: server-2
  servers:
    "M3 7a3 3 0 0 1 3 -3h12a3 3 0 0 1 3 3v2a3 3 0 0 1 -3 3h-12a3 3 0 0 1 -3 -3v-2M3 15a3 3 0 0 1 3 -3h12a3 3 0 0 1 3 3v2a3 3 0 0 1 -3 3h-12a3 3 0 0 1 -3 -3l0 -2M7 8l0 .01M7 16l0 .01M11 8h6M11 16h6",
  // tabler: arrows-split-2
  upstreams:
    "M21 17h-5.397a5 5 0 0 1 -4.096 -2.133l-.514 -.734a5 5 0 0 0 -4.096 -2.133h-3.897M21 7h-5.395a5 5 0 0 0 -4.098 2.135l-.51 .73a5 5 0 0 1 -4.097 2.135h-3.9M18 10l3 -3l-3 -3M18 20l3 -3l-3 -3",
  // tabler: arrows-exchange
  stream: "M7 10h14l-4 -4M17 14h-14l4 4",
  // tabler: certificate
  certificate:
    "M12 15a3 3 0 1 0 6 0a3 3 0 1 0 -6 0M13 17.5v4.5l2 -1.5l2 1.5v-4.5M10 19h-5a2 2 0 0 1 -2 -2v-10c0 -1.1 .9 -2 2 -2h14a2 2 0 0 1 2 2v10a2 2 0 0 1 -1 1.73M6 9l12 0M6 12l3 0M6 15l2 0",
  // lucide: file-text
  logs: "M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2zM14 2v5a1 1 0 0 0 1 1h5M10 9H8M16 13H8M16 17H8",
  // lucide: chart-column
  metrics: "M3 3v16a2 2 0 0 0 2 2h16M18 17V9M13 17V5M8 17v-3",
  // lucide: clipboard-list
  audit:
    "M9 2h6a1 1 0 0 1 1 1v2a1 1 0 0 1 -1 1h-6a1 1 0 0 1 -1 -1v-2a1 1 0 0 1 1 -1zM16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2M12 11h4M12 16h4M8 11h.01M8 16h.01",
  // lucide: history
  history:
    "M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8M3 3v5h5M12 7v5l4 2",
  // tabler: hierarchy-2
  nodes:
    "M10 3h4v4h-4l0 -4M3 17h4v4h-4l0 -4M17 17h4v4h-4l0 -4M7 17l5 -4l5 4M12 7l0 6",
  // lucide: user
  account:
    "M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2M8 7a4 4 0 1 0 8 0a4 4 0 1 0 -8 0z",
  // lucide: settings
  settings:
    "M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915M9 12a3 3 0 1 0 6 0a3 3 0 1 0 -6 0z",
  // lucide: users
  users:
    "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2M16 3.128a4 4 0 0 1 0 7.744M22 21v-2a4 4 0 0 0-3-3.87M5 7a4 4 0 1 0 8 0a4 4 0 1 0 -8 0z",
  // lucide: mail
  notifications:
    "M22 7l-8.991 5.727a2 2 0 0 1-2.009 0L2 7M4 4h16a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2v-12a2 2 0 0 1 2 -2z",
  // lucide: link
  oauth:
    "M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71",
  // lucide: shield-alert
  security:
    "M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1zM12 8v4M12 16h.01",
  // lucide: key-round
  tokens:
    "M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4zM16 7.5a0.5 0.5 0 1 0 1 0a0.5 0.5 0 1 0 -1 0zM16.5 7.5h.01",
  // lucide: clock
  pending: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0zM12 6v6l4 2",
  // lucide: lock
  secrets:
    "M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-7a2 2 0 0 1 2 -2zM7 11V7a5 5 0 0 1 10 0v4",
  // lucide: bell
  bell: "M10.268 21a2 2 0 0 0 3.464 0M3.262 15.326A1 1 0 0 0 4 17h16a1 1 0 0 0 .74-1.673C19.41 13.956 18 12.499 18 8A6 6 0 0 0 6 8c0 4.499-1.411 5.956-2.738 7.326",
  // lucide: sun
  sun: "M8 12a4 4 0 1 0 8 0a4 4 0 1 0 -8 0zM12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41",
  // lucide: moon
  moon: "M20.985 12.486a9 9 0 1 1-9.473-9.472c.405-.022.617.46.402.803a6 6 0 0 0 8.268 8.268c.344-.215.825-.004.803.401",
  // lucide: menu
  menu: "M4 5h16M4 12h16M4 19h16",
  // lucide: x
  close: "M18 6 6 18M6 6l12 12",
  // lucide: search
  search: "M21 21l-4.34-4.34M3 11a8 8 0 1 0 16 0a8 8 0 1 0 -16 0z",
  // lucide: plus
  plus: "M5 12h14M12 5v14",
  // lucide: check
  check: "M20 6 9 17l-5-5",
  // lucide: triangle-alert
  alert:
    "M21.73 18l-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3M12 9v4M12 17h.01",
  // lucide: chevron-right
  chevronRight: "M9 18l6-6-6-6",
  // lucide: chevron-left
  back: "M15 18l-6-6 6-6",
  // lucide: external-link
  external:
    "M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6",
  // lucide: play
  play: "M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z",
  // lucide: refresh-cw
  refresh:
    "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8M21 3v5h-5M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16M8 16H3v5",
  // lucide: copy
  copy: "M10 8h10a2 2 0 0 1 2 2v10a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2v-10a2 2 0 0 1 2 -2zM4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2",
  // lucide: log-out
  signOut: "M16 17l5-5-5-5M21 12H9M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4",
  // lucide: arrow-left
  arrowLeft: "M12 19l-7-7 7-7M19 12H5",
  // lucide: arrow-right
  arrowRight: "M5 12h14M12 5l7 7-7 7",
  // lucide: arrow-down
  arrowDown: "M12 5v14M19 12l-7 7-7-7",
  // lucide: chevron-down
  chevronDown: "M6 9l6 6 6-6",
  // lucide: minus
  minus: "M5 12h14",
  // lucide: activity
  activity:
    "M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2",
  // lucide: scroll-text
  scrollText:
    "M15 12h-5M15 8h-5M19 17V5a2 2 0 0 0-2-2H4M8 21h12a2 2 0 0 0 2-2v-1a1 1 0 0 0-1-1H11a1 1 0 0 0-1 1v1a2 2 0 1 1-4 0V5a2 2 0 1 0-4 0v2a1 1 0 0 0 1 1h3",
  // lucide: route
  route:
    "M3 19a3 3 0 1 0 6 0a3 3 0 1 0 -6 0zM9 19h8.5a3.5 3.5 0 0 0 0-7h-11a3.5 3.5 0 0 1 0-7H15M15 5a3 3 0 1 0 6 0a3 3 0 1 0 -6 0z",
  // lucide: cable
  cable:
    "M17 19a1 1 0 0 1-1-1v-2a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2a1 1 0 0 1-1 1zM17 21v-2M19 14V6.5a1 1 0 0 0-7 0v11a1 1 0 0 1-7 0V10M21 21v-2M3 5V3M4 10a2 2 0 0 1-2-2V6a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2a2 2 0 0 1-2 2zM7 5V3",
  // lucide: shield-check
  shieldCheck:
    "M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1zM9 12l2 2 4-4",
  // lucide: file-clock
  fileClock:
    "M16 22h2a2 2 0 0 0 2-2V8a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 14 2H6a2 2 0 0 0-2 2v2.85M14 2v5a1 1 0 0 0 1 1h5M8 14v2.2l1.6 1M2 16a6 6 0 1 0 12 0a6 6 0 1 0 -12 0z",
  // lucide: server
  server:
    "M4 2h16a2 2 0 0 1 2 2v4a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2v-4a2 2 0 0 1 2 -2zM4 14h16a2 2 0 0 1 2 2v4a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2v-4a2 2 0 0 1 2 -2zM6 6L6.01 6M6 18L6.01 18",
  // lucide: circle-user-round
  circleUserRound:
    "M17.925 20.056a6 6 0 0 0-11.851.001M8 11a4 4 0 1 0 8 0a4 4 0 1 0 -8 0zM2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z",
  // lucide: panel-left-close
  panelLeftClose:
    "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2zM9 3v18M16 15l-3-3 3-3",
  // lucide: panel-left-open
  panelLeftOpen:
    "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2zM9 3v18M14 9l3 3-3 3",
  // lucide: log-in
  logIn: "M10 17l5-5-5-5M15 12H3M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4",
  // lucide: chevrons-up-down
  chevronsUpDown: "M7 15l5 5 5-5M7 9l5-5 5 5",
  // lucide: arrow-up
  arrowUp: "M5 12l7-7 7 7M12 19V5",
  // lucide: arrow-up-right
  arrowUpRight: "M7 7h10v10M7 17 17 7",
  // lucide: arrow-down-up
  arrowDownUp: "M3 16l4 4 4-4M7 20V4M21 8l-4-4-4 4M17 4v16",
  // lucide: corner-down-right
  cornerDownRight: "M15 10l5 5-5 5M4 4v7a4 4 0 0 0 4 4h12",
  // lucide: ellipsis
  ellipsis:
    "M11 12a1 1 0 1 0 2 0a1 1 0 1 0 -2 0zM18 12a1 1 0 1 0 2 0a1 1 0 1 0 -2 0zM4 12a1 1 0 1 0 2 0a1 1 0 1 0 -2 0z",
  // lucide: rotate-ccw
  rotateCcw: "M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8M3 3v5h5",
  // lucide: pencil
  pencil:
    "M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497zM15 5l4 4",
  // lucide: trash-2
  trash2:
    "M10 11v6M14 11v6M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2",
  // lucide: grip-vertical
  gripVertical:
    "M8 12a1 1 0 1 0 2 0a1 1 0 1 0 -2 0zM8 5a1 1 0 1 0 2 0a1 1 0 1 0 -2 0zM8 19a1 1 0 1 0 2 0a1 1 0 1 0 -2 0zM14 12a1 1 0 1 0 2 0a1 1 0 1 0 -2 0zM14 5a1 1 0 1 0 2 0a1 1 0 1 0 -2 0zM14 19a1 1 0 1 0 2 0a1 1 0 1 0 -2 0z",
  // lucide: list-filter
  listFilter: "M2 5h20M6 12h12M9 19h6",
  // lucide: download
  download: "M12 15V3M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5",
  // lucide: upload
  upload: "M12 3v12M17 8l-5-5-5 5M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4",
  // lucide: circle-alert
  circleAlert:
    "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0zM12 8L12 12M12 16L12.01 16",
  // lucide: circle-check
  circleCheck: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0zM16 9l-5.5 5.5L8 12",
  // lucide: circle-x
  circleX: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0zM15 9l-6 6M9 9l6 6",
  // lucide: circle-dot
  circleDot:
    "M11 12a1 1 0 1 0 2 0a1 1 0 1 0 -2 0zM2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0z",
  // lucide: loader-circle
  loaderCircle: "M21 12a9 9 0 1 1-6.219-8.56",
  // lucide: fingerprint
  fingerprint:
    "M12 10a2 2 0 0 0-2 2c0 1.02-.1 2.51-.26 4M14 13.12c0 2.38 0 6.38-1 8.88M17.29 21.02c.12-.6.43-2.3.5-3.02M2 12a10 10 0 0 1 18-6M2 16h.01M21.8 16c.2-2 .131-5.354 0-6M5 19.5C5.5 18 6 15 6 12a6 6 0 0 1 .34-2M8.65 22c.21-.66.45-1.32.57-2M9 6.8a6 6 0 0 1 9 5.2v2",
  // lucide: eye
  eye: "M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0M9 12a3 3 0 1 0 6 0a3 3 0 1 0 -6 0z",
  // lucide: eye-off
  eyeOff:
    "M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49M14.084 14.158a3 3 0 0 1-4.242-4.242M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143M2 2l20 20",
  // lucide: smartphone
  smartphone:
    "M7 2h10a2 2 0 0 1 2 2v16a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2v-16a2 2 0 0 1 2 -2zM12 18h.01",
  // lucide: laptop
  laptop:
    "M18 5a2 2 0 0 1 2 2v8.526a2 2 0 0 0 .212.897l1.068 2.127a1 1 0 0 1-.9 1.45H3.62a1 1 0 0 1-.9-1.45l1.068-2.127A2 2 0 0 0 4 15.526V7a2 2 0 0 1 2-2zM20.054 15.987H3.946",
  // lucide: terminal
  terminal: "M12 19h8M4 17l6-6-6-6",
  // lucide: cpu
  cpu: "M12 20v2M12 2v2M17 20v2M17 2v2M2 12h2M2 17h2M2 7h2M20 12h2M20 17h2M20 7h2M7 20v2M7 2v2M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2v-12a2 2 0 0 1 2 -2zM9 8h6a1 1 0 0 1 1 1v6a1 1 0 0 1 -1 1h-6a1 1 0 0 1 -1 -1v-6a1 1 0 0 1 1 -1z",
  // lucide: memory-stick
  memoryStick:
    "M12 12v-2M12 18v-2M16 12v-2M16 18v-2M2 11h1.5M20 18v-2M20.5 11H22M4 18v-2M8 12v-2M8 18v-2M4 6h16a2 2 0 0 1 2 2v6a2 2 0 0 1 -2 2h-16a2 2 0 0 1 -2 -2v-6a2 2 0 0 1 2 -2z",
  // lucide: hard-drive
  hardDrive:
    "M10 16h.01M2.212 11.577a2 2 0 0 0-.212.896V18a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-5.527a2 2 0 0 0-.212-.896L18.55 5.11A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11zM21.946 12.013H2.054M6 16h.01",
  // lucide: unplug
  unplug:
    "M19 5l3-3M2 22l3-3M6.3 20.3a2.4 2.4 0 0 0 3.4 0L12 18l-6-6-2.3 2.3a2.4 2.4 0 0 0 0 3.4ZM7.5 13.5 10 11M10.5 16.5 13 14M12 6l6 6 2.3-2.3a2.4 2.4 0 0 0 0-3.4l-2.6-2.6a2.4 2.4 0 0 0-3.4 0Z",
  // lucide: globe
  globe:
    "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0zM12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20M2 12h20",
  // lucide: file-diff
  fileDiff:
    "M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2zM9 10h6M12 13V7M9 17h6",
  // lucide: pause
  pause:
    "M15 3h3a1 1 0 0 1 1 1v16a1 1 0 0 1 -1 1h-3a1 1 0 0 1 -1 -1v-16a1 1 0 0 1 1 -1zM6 3h3a1 1 0 0 1 1 1v16a1 1 0 0 1 -1 1h-3a1 1 0 0 1 -1 -1v-16a1 1 0 0 1 1 -1z",
} as const;

export type IconName = keyof typeof PATHS;
export const iconNames = Object.keys(PATHS) as IconName[];

export interface IconProps {
  name: IconName;
  /** 12 is reserved for check marks; 18 remains compatible with existing callers. */
  size?: 12 | 14 | 16 | 18 | 20;
}

export function Icon({ name, size = 16 }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      style={{
        strokeWidth:
          size === 12
            ? "var(--icon-stroke-xs)"
            : size === 14
              ? "var(--icon-stroke-sm)"
              : "var(--icon-stroke)",
      }}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d={PATHS[name]} />
    </svg>
  );
}
