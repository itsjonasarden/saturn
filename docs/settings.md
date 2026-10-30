# Settings

Open them with ⌘, or the gear in the title bar: sections on the left, with a search across all of
them. They're stored in `~/Library/Application Support/Saturn/settings.json`, along with which
panels (sidebar, editor, tickets) were showing, so Saturn opens with the same ones.

- **General**: notifications when an agent needs you while Saturn is in the background; the menu bar
  icon with how many agents wait for you; sharing `.env` and Claude's local files with worktrees; checking for updates now.
- **Appearance**: the theme, Saturn or Cyberdeck (square panels with cut corners on a near-black ground, in Rajdhani); System, Light or Dark; the text size of terminals and files; the plan usage card.
- **Editor**: format on save (with the formatter your project already uses), wrapping long lines,
  line numbers, indent guides, visible whitespace, tab width.
- **Terminal**: Option as Meta (Option+B jumps a word back in the shell) instead of typing the
  characters your keyboard layout puts on it; copy on select. Inside Claude Code, dragging selects
  in Claude itself and copies what you select; hold Shift or Option to select with Saturn instead.
  Dropping files (a screenshot, say) on a terminal types their paths, so the agent attaches them;
  the terminal they'll land in lights up while you drag.
- **Agents**: which agents Saturn found (Claude Code, Codex: installed and logged in) and which one
  new agents start. Saturn's status line under Claude Code's prompt (model, context, the task list's
  step, the session's limit), in Saturn and in any other terminal; turning it on keeps the one you
  had, to put back. What starts each agent, `claude` and `codex` or an alias of your own (`cdsk`, say):
  Saturn types what it needs after it, so a resumed session is `cdsk --resume …`. Codex's status line (its own items: model, folder, branch, context, limits), in
  the Codex sessions Saturn starts. Panes named after Claude's session (a name you type yourself always wins); sessions
  named after tickets, so `/start abc-224` renames the session to `ABC-224`, as `/rename ABC-224`
  would. Tickets mentioned in normal prompts never rename anything.
- **Keyboard shortcuts**: click one and press the keys you want; a shortcut that had them gives them
  up.
- **ClickUp** (optional): your API token, kept in a file only you can read, plus, for each project, the
  commands a ticket starts, one for each agent (`claude -n {ticket}` and `codex` by default;
  `claude -n {ticket} "/start {ticket}"` sends the agent a first message, so it starts working at
  once) and whether it gets its own worktree.
