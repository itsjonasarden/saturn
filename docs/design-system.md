# Design system

Saturn follows the Quarn workflow-automation design (Behance, 2026) with blue in place of its green: near-black and grey almost everywhere, colour only where it has a job, and depth from light and blur instead of decoration.

- **Browsable reference:** the Saturn Design System artifact, https://claude.ai/artifact/RC4SBXYReNJprpjxDNapf3: brand book, tokens in both themes, every component with a live preview, and the full main window.
- **In this repo:** `design/tokens.json` (the tokens, same file as the artifact) and `design/components.css` (the component classes the previews use). `crates/saturn-design` compiles the tokens into Rust constants with a dark and a light value each, so a token changes in one place.

![The Saturn main window](images/saturn-window.png)

## What colour means

| Colour | Means                                                  | Where                                              |
| ------ | ------------------------------------------------------ | -------------------------------------------------- |
| Blue   | Alive: an agent working, focus, the action             | Working dots, the focused pane, the primary button |
| Orange | **Needs you**: a question, a permission prompt, a stop | Pane ring, the tab's dot, the title-bar count      |
| Green  | **Your turn**: the turn finished with nothing pending  | Pane ring, the tab's dot, the title-bar count      |
| Red    | An error, in its own words                             | The tab's dot; the pane counts as needing you      |

Orange and green split "come back" in two: orange is waiting on you, green is done and safe to read.

## Glass and cards

Saturn layers its window the way macOS apps do (Finder, Xcode, Mail):

- **Saturn has its own backdrop.** Behind the cards, the window is Saturn's own gradient (`backdrop` into `backdrop-deep`): a deep navy in dark mode, a soft periwinkle in light, the same over any desktop. The cards stand on it, a little translucent, so they take its tint. (The macOS sidebar material still sits underneath, patched in `vendor/gpui-pre-macos`, but the backdrop covers it.)
- **Content is a card.** The sidebar, a terminal pane or the editor is an opaque **card** (`surface`, radius 10) holding a **strip** of controls and, under one hairline, its content, edge to edge. No well inside a card, so no double outline.
- **What sits on the glass gets a surface.** The title bar's tracks and its secondary button stand on the cards' own surface (`chrome::on_glass`), edged with a shadow ring rather than a border so they keep their height. A bare 5% fill vanished over a light desktop.
- **Edges and fills are relative.** Black in light mode, white in dark, at a low alpha (`theme::overlay`): a 10% separator, 6–11% control fills, a card ring of 7–8%. A fixed grey would read as a sticker on glass; a relative one darkens or lightens whatever is under it. In light mode a card also has a soft shadow; in dark mode a highlight along its top edge.
- **Dialogs** float over a light dim, with the window sharp behind them: no blur.
- **Dark cards are grey, not black** (`surface` is `#1e1e1e`, macOS's own content grey), so they sit on the glass rather than cutting holes in it.
- **Tracks** hold controls that belong together on a faint fill with no border; the pressed item is raised (a white pill with a small shadow in light mode, a lighter pill in dark).

The research behind this, with Apple's measured material values, is summarised in the commit that introduced it.

## Corners, from the window inward

Inner radius = outer radius − inset. The scale starts at the one corner Saturn does not draw, the macOS window (about 16):

| Outer                 | Inset | Inner                       |
| --------------------- | ----- | --------------------------- |
| the window, about 16  | 6     | cards, 10                   |
| card or track, 10     | 4     | controls (tabs, buttons), 6 |
| dialog or palette, 14 | 8     | controls, 6                 |

A test in `crates/saturn-design` fails when one of these stops adding up, and the design system's previews are checked corner by corner with a script.

## Density

Development tools need room for content, so controls are 28px tall, cards sit 6px apart and pad 4px.

## Fonts

Weights: 400 for running text, 500 for every control and name, 300 for big headlines with their point in 600 ("No terminals **yet**"). Nothing else is semibold.

Urbanist for the interface and JetBrains Mono for terminals and code, bundled so Saturn never loads fonts from the network. GPUI draws a variable font only at its default weight, so `crates/saturn/assets/fonts` holds one static file per weight in use (Urbanist 300, 400, 500, 600; JetBrains Mono 400, 700), instanced from the variable fonts with fontTools. Both are OFL 1.1 with no Reserved Font Name.

### Cyberdeck

The second theme (Settings → Appearance → Theme). Its own true-grey neutrals on an opaque near-black ground (near-white in light mode), from the `cyberdeck-dark` and `cyberdeck-light` values in `design/tokens.json`; a token without one uses Saturn's, so the signal colours mean the same in both. Corners are square (`theme::corner`), panels have their top-right corner cut and a 1px edge that takes the status ring's colour, a selection is an ink outline round a faint fill with its bottom-right corner cut, and buttons are the same outline (the primary one in blue). A choice of a few values is a selector (`chrome::selector`): the value between two arrows, over a dash for each value. Focus is ink, not blue. Tabs (`chrome::tab`) are words over a bar, lit under the tab you're on. A switch is the word (On, Off) and a cut box whose block sits right and lit, or left and dim (`chrome::switch`); a menu outlines the line under the pointer; each setting is its own slab, notched along its top edge with its bottom-left corner cut (`chrome::Cut::Row`); the agent and usage cards are grey slabs instead of gradients, and the usage meters are rows of square dots, orange from 80%. The interface font is Rajdhani (500, 600, 700, OFL 1.1, no Reserved Font Name); code stays in JetBrains Mono.
