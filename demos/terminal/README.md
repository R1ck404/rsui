# Terminal

A terminal where the windows are a strip. Each shell gets a column almost as wide as the window, the columns sit side by side
in one row, and the edges of the ones next to the open one show at the sides. `Ctrl+Alt+←` `→` slides from one to the next, and
the overview (`Ctrl+Shift+O`) shrinks them all into a live grid to pick from. rsui draws all of it, the strip, the text and the
cursor, and runs each shell in a pseudo terminal through `pty` in `rsui/desktop` (ConPTY on Windows).

## Running it

From this folder, with `rsui-tsx` installed (see the [root README](../../README.md)):

```
npm install
rsui-tsx app.tsx
```

It opens Windows PowerShell, or the shell you choose in settings, and reopens the columns you had last time.

## What it does

**Layouts** (Settings → Layout, or `Ctrl+Shift+L` to go through them)
- **Normal**, the default: one shell at a time on a card, under a quiet bar with its name and folder, and a dot for each shell
- **Tabs** along the top, as a browser has them
- **Sidebar**: every shell listed down the left with its state and its last command, as a list of threads
- **Strip**: shells side by side as columns that slide (below)
- **Minimal**: the card and nothing else; a bar fades in when the pointer is at the top

In **Tabs** and **Sidebar**, drag a terminal tab onto the open terminal's content. Move toward the left, right, top, or bottom
of a pane to preview the split. Release to move the running shells into the highlighted half, or press Escape to cancel.

**Backgrounds**, drawn behind the card on a canvas of their own: drifting stars, a whale shark made of dots that swims round,
a rippling field of ASCII, halftone dots that swell under the pointer, soft wandering blobs, or digital rain. Each has its
colors, strength, speed and amount, and how see-through the card over it is. The interface font can be the terminal's, to make
it all one piece.

**The strip**
- Columns that are wide, full, two thirds, half or a third of the window, each changed from its header. New ones open to the
  right of the open one, in the folder it's in
- A column can hold a few shells stacked one over another (`Ctrl+Shift+D`), with `Ctrl+Alt+↑` `↓` between them
- The overview (`Ctrl+Shift+O`): every column at once, still running, to click one
- The dots along the bottom: yellow while a command runs, purple when a pane needs attention, and red when its shell has ended
- Use the pane header or the command palette to zoom a stacked pane. The other panes keep running.
- A dropdown terminal: `Ctrl+`` ` from anywhere slides the window down from the top of the screen the pointer is on, as Quake's
  console did, and slides it away again

**Shell integration.** PowerShell, Git Bash and cmd are started with a few lines that mark where each prompt, command and its
output are (OSC 133), and which folder the shell is in. From those:
- A mark beside each command, green when it worked and red when it failed, with how long it took and its exit code
- `Ctrl+Shift+↑` `↓` jumps between prompts, and `Ctrl+Shift+Y` copies the last command's output
- The command you're scrolled up in stays at the top while you read its output
- A command that takes a while tells you when it's done, with a notification if you're in another program
- Commands show on the scrollbar, so a long scrollback can be skimmed
- Click a command mark to select the output. Right-click it for copy, collapse, bookmark, and rerun actions.
  Open **Command history and bookmarks** to filter commands or jump to their output. Rerun requires an empty marked prompt.

**Finding things**
- Search through everything (`Ctrl+Shift+F`), with case and regular expressions, every match marked on the scrollbar
- Hints (`Ctrl+Shift+Space`): every address, file and line, git hash, IP address, UUID and color on the screen gets a letter.
  Typing it copies it; with Shift it opens, a file in your editor at its line
- Copy mode (`Ctrl+Shift+X`): a cursor of its own with vi's keys (`hjkl`, `w` `b` `e`, `0` `$`, `gg` `G`, `v` `V` `Ctrl+V`,
  `/` `?` `n` `N`, `y`) to select and copy without the mouse
- `Ctrl+click` opens a link, or a file at its line and column

**What programs can do.** 24-bit color, the alternate screen, mouse reporting (vim, htop, lazygit), bracketed paste, focus
events, links (OSC 8), the clipboard (OSC 52), titles, notifications (OSC 9, 99 title/body, and 777),
taskbar progress (OSC 9;4), synchronized output, cursor shapes, underline styles, and inline pictures.
**Behavior** lets you choose Kitty graphics, Sixel, iTerm2 `imgcat`, or all three. Kitty graphics supports direct
PNG, RGB, and RGBA transfers with chunking, compression, and ordinary placements. File transfers, shared memory,
animation, and Unicode placeholders are outside its supported set. Kitty keyboard negotiation is on by default.
Application shortcuts can take priority for keys that the terminal program requests.

**Everything else**
- A command palette (`Ctrl+Shift+P`) with every action, ranked as you type
- **Routines** open one named tab per run. Pick a step in the tab's sidebar to see its live shell. Run **Start local development**
  from the palette to try the simulated Web, API and Tests workflow. When the API's first attempt fails, click **Retry API** to
  let Tests start once both services are ready. The previous attempt stays under API in the same tab. Closing the tab ends its
  shells. Run tabs last for the current app session.
- In **Command history and bookmarks**, open a completed command's actions and choose **Add to routine draft** to collect
  commands across panes, or **Save as routine…** to open the editor. Steps in the same phase start together. Each step can wait
  for exit code zero or a literal piece of output. Saved routines use Windows PowerShell, PowerShell 7 or Git Bash and can be
  bound under **Settings → Keyboard**. **Stop routine** releases the panes and cancels queued steps.
- Color schemes from Catppuccin to Tokyo Night to Gruvbox. **Appearance → Customize colors** edits the text, background,
  cursor, selection, window frame and all 16 ANSI colors. Use the color picker or enter a hex value. Edits are saved per scheme and can be reset.
- **Text & cursor** opens the system font picker for installed monospaced fonts. Change text size with `Ctrl+=` `-` or Ctrl and the wheel.
- **Prompt** previews Shell default, Minimal and Powerline styles for new Windows PowerShell and PowerShell 7 tabs.
  **Customize prompt** changes segment colors, path length, separator shape and whether typing starts on a new line.
  Drag a segment by its handle to reorder it, or focus the handle and use the up and down arrow keys.
  Show the shell, folder, Git branch, failed command's exit code, time or username. Powerline separators are drawn by the terminal,
  including separators emitted by your own shell prompt.
- Box drawing and block characters drawn rather than taken from the font, so frames join up at any line height
- A cursor that glides to where it goes, with a trail if you like
- Typing into every live shell in the workspace (`Ctrl+Shift+B)). Each visible recipient shows a broadcast indicator.
- **Record terminal tab** (`Ctrl+Shift+R`) saves the starting screen, timed terminal output, pane layout, resize events,
  and appearance settings in a compressed `.tabrec` file under `terminal-recordings` in your user folder. The recording
  clock pauses while the tab is hidden. Stop with the same shortcut and play it in **Recordings**.
  **Edit recording** opens editing categories on the left, controls on the right, and a timeline below the preview. Drag the
  trim handles, step through frames, crop the picture, or change the theme, colors, and background. **Keystrokes** controls
  which captured keys appear in the preview and export, how long they stay on screen, and how overlapping presses are shown.
  New recordings save typed characters as well as shortcuts and special keys, including input hidden by the terminal.
  **Save edits** keeps edits with the recording.
  **Export MP4** renders the selected range and crop using FFmpeg (available on PATH). Older PNG-frame recordings still play
  in **Recordings**.
- **Record shell output (.cast)** in the command palette keeps a portable asciinema recording. **Instant replay of this pane**
  opens the recent in-memory shell output. **Behavior** sets its duration and the default speed.
- A test card from the palette, to see colors, styles, wide characters and box drawing at a glance
- Mica or Acrylic behind the strip, the strip's tint and the columns' opacity, corner radius, how much of the neighbors shows

**Behavior** also includes prompt-aware paste warnings, per-pane host, Git branch and command time, and regex output
triggers. Trigger actions are Highlight, Notify, and Needs attention. Enable **Match as text arrives** for prompts
that ask for input without printing a newline. **Shells** lets you enable or refresh profiles from SSH aliases and
installed WSL distributions. **General** can restore bookmarks and 250 or 1,000 recent lines of plain-text output per pane
when the workspace is reopened. It launches fresh shells.

Protocol and workflow references: [Kitty keyboard](https://sw.kovidgoyal.net/kitty/keyboard-protocol/),
[Kitty graphics](https://sw.kovidgoyal.net/kitty/graphics-protocol/), [DEC Sixel](https://vt100.net/dec/ek-vt3xx-hr-002.pdf),
[iTerm2 triggers and replay](https://iterm2.com/features.html), [Warp block actions](https://docs.warp.dev/terminal/blocks/block-actions/),
[cmux notifications](https://github.com/manaflow-ai/cmux), [Windows Terminal profiles](https://learn.microsoft.com/windows/terminal/customize-settings/startup),
and [asciicast v2](https://docs.asciinema.org/manual/asciicast/v2/).

## Checking it without a window

With `--do`, no shell starts: `pty out TEXT` makes one print (with `\e` for escape) and `pty written` says what was typed into it.
`RSUI_PTY=real` starts real shells even then. The app leaves `__terminal` on the global object for `eval` steps:
`__terminal.run('overview')` runs a command from the palette by its id, and `__terminal.workspace` is the strip.

The emulator, search, hints, keys and the strip's layout don't need rsui, and their tests run under Node 24:

```
npm test
```

## Files

| File | What's in it |
|---|---|
| `app.tsx` | The window: the title bar, the strip, the status bar and what opens over them, and where each key goes |
| `src/vt/` | The emulator (`terminal.ts`), what keys and the mouse send (`keys.ts`), styles and characters' widths |
| `src/term/Session.ts` | One shell: its pty, its emulator, scrolling, selection, search, hints, copy mode and recording |
| `src/term/TerminalPane.tsx`, `Row.tsx` | Drawing a shell: rows of text runs, the cursor, hints, the scrollbar, and the mouse |
| `src/term/boxdraw.ts` | Box drawing and block characters as rectangles |
| `src/term/copymode.ts`, `hints.ts`, `search.ts`, `recording.ts` | What their names say |
| `src/strip/` | The strip and the overview, and where each column goes (`layout.ts`) |
| `src/workspace.ts` | The columns, opening and closing them, and keeping them for next time |
| `src/commands.ts` | Every action, its keys and its name in the palette |
| `src/shells.ts`, `src/launch.ts` | The shells there are and the integration script each is started with |
| `src/quake.ts` | The dropdown terminal |
| `src/ui/` | The title bar, palette, search bar, settings, recordings and small controls |
| `test/` | Tests for the parts that don't need rsui |
