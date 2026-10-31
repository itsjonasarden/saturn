# FAQ

**Is Akyba an IDE?** No. It's where the agents run, with enough of an editor to read and fix what
they touch. Keep your own editor for the rest.

**Does it change my Claude Code or Codex setup?** No. Akyba loads its small plugin only for the
`claude` sessions started inside it, and runs the Codex sessions it starts with a profile of its own
beside your config (see [Claude Code and Codex](agents.md)). The Claude status line in Settings is
the one thing that reaches other terminals, and only if you turn it on.

**Which agents does it work with?** Claude Code and Codex, each fully: states, approvals, resume,
sessions, limits. Any other program runs in its terminals too, without the signals.

**What leaves my Mac?** Akyba has no server and collects nothing. It asks GitHub for new
versions and, through your own `gh`, for pull requests; Claude Code for your plan's usage (Codex's it
reads from Codex's own files); and ClickUp only if you connect a token, which stays in a file only
you can read.

**Why does macOS warn me the first time?** Akyba isn't notarised, because that needs a paid Apple
Developer ID. It's built from this repo, and updates are signed, so an installed Akyba only accepts
versions published here.

**Is it free?** Yes, under the MIT licence. The agents need your own Claude or ChatGPT plan.

**Windows or Linux?** No, Akyba is made for macOS.
