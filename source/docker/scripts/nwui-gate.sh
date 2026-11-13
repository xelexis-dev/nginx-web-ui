#!/bin/sh
# gate-outcome validates the root-owned selection; never interpret JSON as shell.
nwui_gate_outcome() {
  nwui_outcome=$(/usr/local/bin/nwui-api gate-outcome) || exit 2
  case "$nwui_outcome" in
    pg17|upgrading|postponed|refused-preflight|refused-fresh|refused-privilege|restored|restore-incomplete|activation-failed) ;;
    *) echo 'storage_selection_invalid' >&2; exit 2 ;;
  esac
}
nwui_gate_oneshot() {
  nwui_gate_outcome
  [ "$nwui_outcome" = pg17 ] || exit 0
}
nwui_gate_longrun() {
  nwui_gate_outcome
  [ "$nwui_outcome" != pg17 ] || return 0
  if [ -f notification-fd ]; then
    read -r nwui_notification_fd < notification-fd
    case "$nwui_notification_fd" in [0-9]) ;; *) echo 'notification_fd_invalid' >&2; exit 2 ;; esac
    eval "printf '\n' >&$nwui_notification_fd; exec $nwui_notification_fd>&-" || exit 2
  fi
  exec /command/s6-pause
}
