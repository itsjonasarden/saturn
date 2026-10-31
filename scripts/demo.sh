#!/bin/sh
# `pnpm demo`: Akyba on a made-up project, to look around or show it off. Agents in every state,
# ClickUp tickets in the right panel (made up, never sent anywhere), the database diagram, the
# history, uncommitted changes and their impact, a type error for the language server.
#
# Nothing of yours is touched: the demo has its own settings folder and its own project, in
# $TMPDIR/akyba-demo, made again on every run.
set -eu
cd "$(dirname "$0")/.."

home="${TMPDIR:-/tmp}/akyba-demo"
sh scripts/demo/make-project.sh "$home"
cargo build --release -p akyba
sh scripts/sign-local.sh target/release/akyba

AKYBA_DEMO_SHELL="${SHELL:-/bin/zsh}" \
SHELL="$PWD/scripts/demo/shell.sh" \
AKYBA_USER_DATA="$home/settings" \
AKYBA_PROJECT="$home/acme-shop" \
AKYBA_MORE_REPOS="$home/acme-docs" \
AKYBA_PANES=4 \
AKYBA_FAKE_AGENTS=1 \
AKYBA_FAKE_TICKETS=1 \
AKYBA_FAKE_USAGE=1 \
AKYBA_OPEN_FILE="$home/acme-shop/packages/money/src/tax.ts" \
  exec target/release/akyba
