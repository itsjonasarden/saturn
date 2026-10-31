# Design system

Saturn follows the Quarn workflow-automation design (Behance, 2026) with blue in place of its green: near-black and grey almost everywhere, colour only where it has a job, and depth from light and blur instead of decoration.

- **Browsable reference:** the Saturn Design System artifact, https://claude.ai/artifact/RC4SBXYReNJprpjxDNapf3: brand book, tokens in both themes, every component with a live preview, and the full main window.
- **In this repo:** `design/tokens.json` (the tokens, same file as the artifact) and `design/components.css` (the component classes the previews use). `crates/saturn-design` compiles the tokens into Rust constants with a dark and a light value each, so a token changes in one place.

![The Saturn main window](images/saturn-window.png)

## How it's built

Four layers, each using only the one below it:

1. **Tokens**, `design/tokens.json`: colours (a dark and a light value for every theme), tones (how strong a relative fill or stroke is), the spacing scale, radii, shadows, fonts, the type scale, icon sizes, sizes (controls, rows, dots, the title bar, dialogs), and the themes.
2. **`crates/saturn-design`** compiles them into constants: `color::INK`, `tone::EDGE`, `space::SPACE_3`, `text::LABEL`, `icon::SM`, `size::ROW`, `THEMES`. Nothing in it is written by hand.
3. **Components**, `crates/saturn/src/chrome.rs`: `button`, `icon_button`, `corner_button`, `track` and `track_item`, `tab` and `tab_row`, `selector`, `switch`, `input`, `panel` and `card`, `floating`, and the menu in `menu.rs`. A component reads the theme (`chrome::look()`) and draws the version it asks for.
4. **Views** put components together. A view never writes a size, a spacing or a radius of its own, and never asks which theme is on.

`crates/saturn/tests/design_rules.rs` holds the views to that, in `pnpm health`:

- text sizes come from the type scale: `.text_scale(text::META)`, not `.text_size(px(12.))`;
- spacing comes from the spacing scale: `.gap(len(space::SPACE_1))`, not `.gap(px(4.))`;
- line heights come from the type scale: `.line_height(px(text::META.line_height))`;
- corners come from the theme: `.rounded(theme::corner(radius::RADIUS_SM))`;
- fills and strokes come in the tones: `theme::tone(tone::HOVER)`, not `overlay(0.06)`;
- icons come in the icon sizes: `chrome::icon(name, icon::SM, …)`;
- nothing compares theme names.

A value off a scale that a shape needs (a dot lined up with the first line of text, a badge's padding) says why on its line, after `// off-scale:`. Components and `theme.rs` draw shapes, so they may write pixels.

**The gallery** shows every component in every state, in the theme and appearance Settings has: `pnpm gallery`. With `SATURN_GALLERY=1 SATURN_SCREENSHOT=…` it is pictured offscreen like any spike, so a component's change can be seen in every theme side by side before it ships.

### Themes

A theme is a description in `design/tokens.json` (under `themes`), not code: its name, font, corners (round, or cut by `panelCut` and `controlCut`), the gap between cards, the colour of focus, and which version of each control it draws: icons (round or sharp), title bar, tabs, selection, choices, switch, buttons, menus, setting rows, meters, agent cards and the editor's current line. `themes.options` lists every version with what it looks like. Settings lists the themes from the same file.

To add a theme:

1. Add an entry to `themes.themes` with a new `id`, choosing a version of each control.
2. Give the colours that differ an `<id>-dark` and an `<id>-light` value; the rest borrow the first theme's.
3. If it needs a font, add its family under `type.families` and its files to `crates/saturn/assets/fonts`.
4. Look at it with `pnpm gallery`, then in the app.

A new version of a control (a third kind of tab, say) is a new option in `themes.options`, a variant in `saturn-design`, and its drawing in that one component.

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

Development tools need room for content, so controls and list rows are 28px tall (`size::CONTROL`, `size::ROW`), cards sit 6px apart (4 in Cyberdeck) and pad 4px. Dialogs come in three widths, `size::DIALOG_SMALL` (a field and a few words), `DIALOG_MEDIUM` (a list) and `DIALOG_LARGE` (two panes or a table); marks are `size::DOT`, and `DOT_LARGE` for an agent's state. Tones run from `wash` (3%) to `outline-strong` (40%), each named for its job; icons are 11, 13, 14 or 16px.

## Fonts

Weights: 400 for running text, 500 for every control and name, 300 for big headlines with their point in 600 ("No terminals **yet**"). Nothing else is semibold.

Urbanist for the interface and JetBrains Mono for terminals and code, bundled so Saturn never loads fonts from the network. GPUI draws a variable font only at its default weight, so `crates/saturn/assets/fonts` holds one static file per weight in use (Urbanist 300, 400, 500, 600; JetBrains Mono 400, 700), instanced from the variable fonts with fontTools. Both are OFL 1.1 with no Reserved Font Name.

### Cyberdeck

The second theme. Its own true-grey neutrals on an opaque near-black ground (near-white in light mode); its signal colours are its own, saturated: neon on the dark ground, deep on the light, meaning what Saturn's mean (blue working, orange needs you, green your turn, red errors). Square panels, their top-right corner cut, with a 1px edge that takes the status ring's colour; cards 4px apart. A selection is an ink outline round a faint fill, its bottom-right corner cut, and buttons are the same outline (the primary one blue, its words blue too). Tabs are words over a bar; a choice is a selector, the value between two arrows; a switch is the word and a cut box. Focus is ink, not blue; the editor's current line is marked by its number alone. Each setting is its own notched slab, agent and usage cards are grey slabs, usage meters are rows of square dots. Its icons are Saturn's own Lucide icons drawn sharp: square line ends, mitred joins, square corners and square dots, made from the same drawings as they load (`saturn_core::sharp_icon`). Where Lucide has an angular icon that means the same as a round one, Cyberdeck draws that instead (`iconSwaps`): the square key, the stacked server for a table, square checks and crosses, the bracketed magnifier. Rajdhani (500, 600, 700, OFL 1.1, no Reserved Font Name) for the interface; code stays in JetBrains Mono.
