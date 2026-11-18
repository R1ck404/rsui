# A long list of page elements

What a list of thousands of rows costs when it's made the way a page makes one: rows of Tailwind-styled `<div>`s in a `<div>`
that scrolls (`overflow-y-auto`), inside `PageWindow`, with name, size and date columns as the file explorer's details view has
them. The whole list renders again whenever the selection moves, as it would in React, with each row in `memo`.

`app.tsx` is the list. `LONG_LIST_N` says how many rows (1,000 unless it says), `LONG_LIST_MODE=cv` gives each row
`content-visibility: auto; contain-intrinsic-size: auto 28px` (`.row-cv` in `app.css`), and `LONG_LIST_MODE=native` makes the same
rows of rsui's own `<row>` and `<text>` in a `<list>`, which only builds what shows, to compare with. Up, Down, Home, End, Page Up
and Page Down move the selection, which `scrollIntoView({ block: 'nearest' })` brings into view, as a file list does.

## Skipping rows by themselves

Measured on 2026-11-18 as below, with what's described in DESIGN.md (rows far from being seen skipped by themselves, each row
compared once, what scrolls keeping its rows, scrolling as a row changes moving the rows rather than laying them out again): before
is the commit this started from (b84a6192), after is with it, each run twice, one after the other, with nothing else running. Down
is the median of 17 presses that move the selection within what shows, and Down, scrolling, of 40 that each scroll the list a row,
from `steps-down.txt` (22 presses, a wait, then 40 more), run as `steps.txt` is, beside it.

| Rows | | Before | After | Native `<list>` |
|---|---|---|---|---|
| 1,000 | starting | 695-745 ms | 717-1,011 ms | 107 ms |
| 1,000 | memory after | 86 MB | 80 MB | 10 MB |
| 1,000 | Down | 3.1 ms | 2.3 ms | 0.5 ms |
| 1,000 | Down, scrolling | 5.5 ms | 3.0 ms | 0.5 ms |
| 5,000 | starting | 3,212-3,241 ms | 1,005-1,014 ms | 106 ms |
| 5,000 | memory after | 352 MB | 181 MB | 13 MB |
| 5,000 | Down | 9.1 ms | 3.9 ms | 0.5 ms |
| 5,000 | Down, scrolling | 20 ms | 6 ms | 0.5 ms |
| 5,000 | End | 105-109 ms | 5.1-5.4 ms | 0.8 ms |
| 25,000 | starting | 17,153-17,214 ms | 4,965-5,116 ms | 114 ms |
| 25,000 | memory after | 1,765 MB | 850 MB | 18 MB |
| 25,000 | Down | 83 ms | 12.5-14 ms | 0.5 ms |
| 25,000 | Down, scrolling | 186-188 ms | 19-23 ms | 0.5 ms |
| 25,000 | End, Home | 165-214 ms | 17-38 ms | 0.8 ms |
| 25,000 | wheel | 40-71 ms | 4.5-15 ms | 1.3-1.4 ms |

Lists of fewer than 2,000 are drawn whole, as before: the 1,000-row numbers are from what renders them again costing less. Where
a step had a garbage collection in it, it's slower, by 10-40 ms at 25,000 rows (the slowest Down was 157 ms), which comes with
how much there is: what's left per row is its own element styled and made, and the app making 25,000 of them each time the list
renders. With `content-visibility: auto` said on each row, after: starting 5.0 s, Down 13 ms, Down, scrolling, 19 ms at 25,000.
The native list costs what shows, whatever the count.

## Sharing what's the same

Measured on 2026-11-18 with what DESIGN.md says of page elements sharing what's the same: each element's handle with only what's its
own, a long list's rows of a kind starting from one look, fonts shared, and parts and contexts left out where nothing uses them.
Before is 285452a9, after is with it, run one after the other three times each at each count, with no window and nothing else
running but one build, as `steps.txt` is but with 15 Downs: starting is how long the process took apart from the steps, which is
mostly the first render, and memory is the process's peak private bytes. Each is the median of the three. Down at 25,000 is also
the median of 51 presses that move within what shows, and Down, scrolling, of 120 that each scroll a row, from three runs of
`steps-down.txt` each.

| Rows | | Before | After |
|---|---|---|---|
| 1,000 | starting | 968 ms | 927 ms |
| 1,000 | memory | 79 MB | 75 MB |
| 1,000 | Down | 2.3 ms | 2.3 ms |
| 5,000 | starting | 1,080 ms | 718 ms |
| 5,000 | memory | 168 MB | 116 MB |
| 5,000 | Down | 3.5 ms | 3.5 ms |
| 25,000 | starting | 5,654 ms | 3,397 ms |
| 25,000 | memory | 759 MB | 497 MB |
| 25,000 | Down, of 15 | 15.7 ms | 16.5 ms |
| 25,000 | Down, of 51 | 19.4 ms | 18.4 ms |
| 25,000 | Down, scrolling | 27.2 ms | 25.5 ms |

What each row still costs, at 25,000 (about 18 KB of the process, of which 8.5 KB is objects in JavaScript): the app's own `Row` and its JSX, which
makes six elements, each `<span>`'s copied once more to be given a key by where it is (`joinedText`); the row's `Styled`
component, with its hooks, effects, what it read of what's above it and what it is to the page (`live`, `me`); its look as
what it's drawn with, about 40 props in JavaScript and again in Rust as the host element's (`Props::replace_from`, about 800 B a
row); and the text it's named by. A component for each element is most of that, and what's left to take away next.

## Page elements drawn in place

Measured on 2026-11-18 with what DESIGN.md says of a page's plain elements drawn in place: each row's `<div>` and `<span>`s are
host elements of the app's `Row`, with no component, hooks or effects of their own, and a row that skips what's in it keeps that as
it was given. Before is bcaeea55, after is with it, run one after the other twice each with `steps.txt`, and Down at 25,000 from
five runs of `steps-down.txt` each, on a machine other builds were using. Memory is private bytes.

| Rows | | Before | After |
|---|---|---|---|
| 1,000 | starting | 774-908 ms | 680-1,015 ms |
| 1,000 | memory to start | 63-66 MB | 51 MB |
| 5,000 | starting | 793-834 ms | 657-658 ms |
| 5,000 | memory to start | 95 MB | 73 MB |
| 25,000 | starting | 3,545-4,062 ms | 2,883-2,896 ms |
| 25,000 | memory to start | 418-423 MB | 305 MB |
| 25,000 | memory after the steps | 568-579 MB | 490 MB |
| 25,000 | Down, median of 21 a run | 13-24 ms | 10.5-20 ms |
| 25,000 | Down, scrolling, median of 40 a run | 20-33 ms | 17-32 ms |

What a row of 25,000 still takes, by the engine's count (`census` in a build with `alloc-owners`): the elements the app's JSX makes
for it, the row's `live` with its look worked out (about 550 B, and the look itself about 460 B), what stands for its `<div>` among
the components, and its own `Row`. Most of starting is still each row's look being worked out once and its `Row` rendering.

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
