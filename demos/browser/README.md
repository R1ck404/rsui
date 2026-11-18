# Browser

A calm, minimal web browser built with rsui and Tailwind CSS 4, laid out like Arc: your tabs live in a sidebar on a softly tinted
frame, and the page sits beside it as a white card. rsui draws all of the browser itself, the sidebar, command bar, start page,
settings and first-run tour. Each page is shown by the system's browser view, WebView2 (Edge's runtime, which comes with
Windows 11), through rsui's `<webview>` element.

## Running it

From this folder, with `rsui-tsx` installed (see the [root README](../../README.md)):

```
npm install
rsui-tsx app.tsx
```

The first run opens a short tour that sets it up as you go: light or dark, the space's color, the search engine, a few favorites,
and the shortcuts. `rsui-tsx --pack browser.exe app.tsx` makes a single program you can hand to someone.

## What it does

- **A sidebar of tabs.** The page's address sits at the top, with favorites as tiles under it. Below are the space's pinned tabs,
  then today's tabs. Right-click a tab to pin it, add it to favorites, copy its link, duplicate it, move it to another space,
  put it to sleep or close it
- **A command bar** (`Ctrl+T`, or `Ctrl+L` to change where the open tab goes): type an address or a search. Suggestions come from
  your open tabs (it switches to them), favorites and history, ranked by match, how often and how lately. `Alt+Enter` opens the
  choice in a new tab
- **Spaces,** each with its own tabs and color, which tints the window while it's open. A space can keep its sign-ins apart, with
  its own cookies and storage, so a work account stays out of a personal one. Switch with the dots at the bottom of the sidebar
  or `Ctrl+Alt+←` `→`
- **A start page** for new tabs: a greeting, search, your favorites and where you've been lately, drawn by rsui so it's there
  at once
- **The sidebar hides** with `Ctrl+S`. The page then has the whole window, and the sidebar slides back over the page while the
  pointer is at the window's edge
- **Sleeping tabs.** A tab you haven't looked at for a while lets its page go and keeps only its address. It loads again when
  you open it. You choose how long that takes, or turn it off
- **Its own error pages**, for a site that can't be found, a connection that isn't private, being offline, or a page that stopped
  working, each with Try again
- **Full screen** follows the page, so a video that goes full screen takes the whole window
- **Its own title bar,** with the window's buttons in the corner and Windows 11's Mica or Acrylic showing through the frame
- **Settings** (`Ctrl+,`): search engine, what a new tab opens, the whole address or only the site, when tabs sleep, theme, frame
  material, tint, corners, which side the sidebar is on, animations, spaces, history, cookies, and the tour again

## Shortcuts

They work while a page has the keyboard too: the page's view hands them to the browser, and the page never sees them.

| Keys | Does |
|---|---|
| `Ctrl+T` | New tab: the command bar, or the start page if settings say so |
| `Ctrl+L`, `F6` | Change where the tab goes |
| `Ctrl+W`, `Ctrl+Shift+T` | Close the tab, reopen the last one closed |
| `Ctrl+Tab`, `Ctrl+Shift+Tab`, `Ctrl+1` … `9` | Next, previous, or a tab by number (`9` is the last) |
| `Alt+←`, `Alt+→`, `Ctrl+R`, `F5` | Back, forward, reload |
| `Ctrl+S` | Show or hide the sidebar |
| `Ctrl+D` | Pin or unpin the tab |
| `Ctrl+Alt+←` `→` | Previous or next space |
| `Ctrl+Shift+C` | Copy the address |
| `Ctrl+Shift+I` | The page's developer tools |
| `Ctrl+,` | Settings |

## How the pages are shown

`<webview>` is one of rsui's own elements, like `<list>`: CSS places and sizes it, and it's rounded like the card it's in.
The browser view is a window of its own over rsui's. rsui keeps it where the element is, cuts it to what shows, and cuts out
whatever rsui draws on top of it, such as the command bar, settings or the sidebar sliding over. Nothing of it is loaded until
the first page is shown. Pages keep their cookies and storage in the app's own folder, never in your Edge profile.

Every tab keeps its view while you're using it, hidden when it isn't the open one, so switching back is instant. Asleep, a tab
has no view at all.

## Checking it without a window

With `--do`, no browser is started. A page "loads" as soon as it's asked for, titled with its host, and `webview` steps make it
do what a real page would: `webview title TEXT`, `webview load URL`, `webview popup URL`, `webview key Ctrl+T`,
`webview fail URL -105`, `webview fullscreen on`. `checks/smoke.txt` goes through the tour, opening, closing and reopening
tabs, suggestions, an error page, the sidebar, settings and spaces:

```
rsui-tsx --do @checks/smoke.txt app.tsx
```

Run it with `APPDATA` and `LOCALAPPDATA` pointed at an empty folder so it starts fresh, with the tour.

## Files

| File | What's in it |
|---|---|
| `app.tsx` | The window: the title bar, sidebar, the page's card and what's over it, and what each shortcut does |
| `src/useBrowser.ts` | The browser's state and everything that changes it: settings, spaces and tabs, history, which pages are awake, their views |
| `src/session.ts` | What can be done to spaces, tabs and favorites, as plain functions from one session to the next |
| `src/model.ts` | What's kept between runs: settings, spaces and tabs, favorites, history |
| `src/sidebar/` | The sidebar: the address, favorites, tab rows, the spaces along the bottom, and the handle that resizes it |
| `src/command/` | The command bar, and how it ranks what it suggests (`suggest.ts`) |
| `src/page/` | A tab's page and its `<webview>`, the start page, the error pages, and the card that holds them |
| `src/onboarding/` | The first-run tour, a file for each step |
| `src/settings/` | Settings, a file for each section |
| `src/TitleBar.tsx`, `src/ui/` | The title bar, the window's buttons, and small controls: icon buttons, site icons, switches, segmented choices, keys |
| `src/menus.ts` | The right-click menus of tabs, favorites and spaces |
| `src/useShortcuts.ts`, `src/keys.ts` | The shortcuts, from the window and from inside pages |
| `src/useWindowLook.ts`, `src/theme.ts`, `app.css` | Colors from the theme and the space as CSS variables for Tailwind, the frame's material, the title on the taskbar |
| `src/url.ts` | Addresses and searches, search engines, and how addresses are shown |
