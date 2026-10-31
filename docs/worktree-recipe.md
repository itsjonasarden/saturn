# Worktree recipe

A fresh worktree has the code but nothing git ignores: no dependencies, no builds. With no setup
at all, Akyba guesses the steps from the repo's own files: the lockfile says how to install
(`pnpm-lock.yaml`, `yarn.lock`, `package-lock.json`, `bun.lock`, `uv.lock`, `poetry.lock`,
`Gemfile.lock`, `composer.lock`, `mix.lock`, `go.sum`), and `dev` scripts (at the top, or one per app
in `apps/`, `services/`, `packages/`) or `bin/dev` say how to start it.

To change the steps, "Create worktree recipe" (the repo's ⋯) saves the guess to
`.akyba/worktree.json`, to adjust and commit with the repo:

```json
{
  "setup": ["pnpm install", "pnpm --filter @acme/shared build"],
  "run": { "api": "pnpm --filter api dev", "web": "pnpm --filter web dev" },
  "cleanup": "docker compose down",
  "checks": { "lint": "pnpm lint", "test": "pnpm test" }
}
```

- `setup` runs in a worktree Akyba makes for a ticket, before the agent starts (and the agent starts
  only if it worked). "Set up this worktree" (the pane's ⋯) runs it again, or in a worktree made
  some other way.
- `run` names the commands that start the app. "Start the app" opens a tab for each, named after it.
- `cleanup` runs in a worktree before Worktrees removes it (stopping its containers, say).
- `checks` names commands that check the work (`"lint": "pnpm lint"`, `"test": "pnpm test"`; guessed
  from `package.json` scripts or Cargo without a recipe). "Run a check" (the pane's ⋯) runs one, or all, in
  the background: the pane shows each as it runs and how it went, a click opens its whole log, and a
  failed one goes to the agent as a short summary with the log's path. Output that says no tests
  ran counts as a failure, whatever the exit code.

Files git ignores that worktrees should still get (local config, certificates) go in
`.worktreeinclude`, written like `.gitignore`, as Claude Code's `claude --worktree` does: Akyba copies them from the
main checkout into a worktree it makes, and "Set up this worktree" fills in any that are missing.

Each checkout also gets its own block of ten ports, the same every time: `AKYBA_PORT` in its
terminals is the first (42000, 42010…), and the sidebar shows the block. Use it where two worktrees'
servers would otherwise clash; shell arithmetic gives the rest:

```json
"run": {
  "api": "PORT=$((AKYBA_PORT + 1)) pnpm --filter api dev",
  "web": "API_URL=http://localhost:$((AKYBA_PORT + 1)) pnpm --filter web dev --port $AKYBA_PORT"
}
```

A list of steps runs one after another and stops at the first that fails. Every field is optional. A
branch made before the recipe existed uses the main checkout's. Steps Akyba guessed say so in the
pane before they run.
