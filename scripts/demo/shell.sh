#!/bin/sh
# The shell `pnpm demo` gives its panes: each of the first four shows something an agent might
# have printed, then hands over to your own shell, so the panes stay real terminals.
sh "$(dirname "$0")/pane.sh"
exec "${AKYBA_DEMO_SHELL:-/bin/zsh}" -l
