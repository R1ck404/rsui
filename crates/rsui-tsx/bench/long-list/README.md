# A long list of page elements

What a list of thousands of rows costs when it's made the way a page makes one: rows of Tailwind-styled `<div>`s in a `<div>`
that scrolls (`overflow-y-auto`), inside `PageWindow`, with name, size and date columns as the file explorer's details view has
them. The whole list renders again whenever the selection moves, as it would in React, with each row in `memo`.

`app.tsx` is the list. `LONG_LIST_N` says how many rows (1,000 unless it says), and `LONG_LIST_MODE=cv` gives each row
`content-visibility: auto; contain-intrinsic-size: auto 28px` (`.row-cv` in `app.css`). Up, Down, Home, End, Page Up and Page Down
move the selection, which `scrollIntoView({ block: 'nearest' })` brings into view, as a file list does.

## Running it

It needs Tailwind: `npm install` here, or a junction to another `node_modules` with `tailwindcss` 4 in it, as the file explorer's
(`mklink /J node_modules ..\..\..\..\demos\file-explorer\node_modules`, removed with `rmdir`).

```
powershell -File run.ps1 -Exe path\to\rsui-tsx.exe -Counts 1000,10000,25000 [-Mode cv] [-Out folder]
```

runs `steps.txt` with no window, sandboxed, once for each count, and says how long each step took, from the key going down to
what it changed laid out and drawn (`RSUI_STEP_TIMES`), the process's memory before and after, and how long the rest took, which
is mostly starting: rendering, styling and laying out every row for the first time. With `RSUI_SPLIT` each step's output in
`-Out` also says where its time went, part by part: `js`, `gc`, `flush`, `layout`, `paint` and what the page layer asked of Rust.
The step times include what the harness does itself, as finding what changed to print it, which a window doesn't.

Times vary a lot with what else the machine's doing, so compare builds by running them one after the other, more than once.

## What was measured

On a Ryzen 7800X3D, release builds (`lto=false`, 16 codegen units), on 2026-11-18, with other builds running beside them. Before is
the commit this started from; after is with what's described below. In ms, for one step each; where a step ran twice, both are
given.

| Rows | Step | Before | After | After, rows `content-visibility: auto` |
|---|---|---|---|---|
| 1,000 | Down | 23, 21 | 3.4, 3.0 | 3.8, 2.8 |
| 1,000 | End | 31, 49 | 6.1, 5.6 | 26, 4.9 |
| 1,000 | wheel | 0.9-1.2 | 1.0-1.1 | 0.6-8.5 |
| 1,000 | starting | 1,049 | 1,026 | 284 |
| 1,000 | memory after | 102 MB | 85 MB | 46 MB |
| 10,000 | Down | 398, 196 | 27, 23 | 26, 14 |
| 10,000 | End | 430, 432 | 51, 56 | 55, 34 |
| 10,000 | Page Down | 432 | 51 | 92 |
| 10,000 | wheel | 14-15 | 10-12 | 5-14 |
| 10,000 | starting | 6,614 | 6,603 | 1,906 |
| 10,000 | memory after | 976 MB | 730 MB | 317 MB |
| 25,000 | Down | 748, 937 | 81, 79 | 36, 67 |
| 25,000 | End | 1,086, 1,088 | 176, 190 | 110, 89 |
| 25,000 | Home | 1,279 | 171 | 88 |
| 25,000 | Page Down | 1,504 | 182 | 88 |
| 25,000 | wheel | 49-53 | 61-73 | 13-21 |
| 25,000 | starting | 17,156 | 16,914 | 4,780 |
| 25,000 | memory after | 2,226 MB | 1,844 MB | 796 MB |

Memory is private bytes at the end. The wheel at 25,000 rows moved between runs by more than the difference shown here: run one
after the other 15 times each, the median of what it did came to 64-81 ms before and 47-74 ms after.

What made the difference, without anything said in CSS:

- **A list rendered again isn't looked through again.** `keptKids` in `web.js` keeps what a block worked out about what's in it,
  as whether any of it goes on a line, so a long list rendered again only looks at what's new. It looked for the kind of row at
  the wrong place in what it kept, so it never kept anything, and it only kept what was the same object, which nothing React
  renders again is. Now it does, and a row made again with the same kind, key and props is the same row to it. That was a call
  into JavaScript for every row, and rules matched against every row, each time the selection moved.
- **Laying out a list again lays out no rows.** A scrolling area with a scrollbar that takes room laid what's in it out at its
  full width first and then again at the width the scrollbar leaves, every time: every row twice (`BAR_DOWN` in rsui-layout). And
  scrolling to the far end from code moves the rows rather than laying them out again.
- **`scrollIntoView()` doesn't make the whole page again**, only what's around the row, and asking whether the list can have the
  keyboard looks at its `tabIndex` before looking through all its rows.
- **How far a long list goes is worked out once** for each layout rather than each time it's asked (`info_of` in rsui-layout).

What's still as many rows as there are: rendering the list itself (the app makes 25,000 elements), the runtime going through
them, laying them out when one changes size, how far they go after each scroll, and, without `content-visibility`, styling and
making each row as it starts, which is most of starting and of the memory, and the garbage collections that follow from that.
