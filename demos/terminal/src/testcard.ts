// A test card, written straight onto the screen (the shell never sees it): every way text can look, so it's plain at a glance
// whether colors, styles, wide characters and box drawing all come out right.

const E = '\x1b[';

export function testCard(cols: number): string {
    const out: string[] = [];
    const line = (s = '') => out.push(s + `${E}0m\r\n`);
    const width = Math.max(20, Math.min(cols - 2, 96));
    line(`${E}1m Test card ${E}0m${E}2m  colors, styles, Unicode and box drawing`);
    line();
    // Sixteen colors, as text and as backgrounds.
    let row = ' ';
    for (let i = 0; i < 8; i++) row += `${E}${30 + i}m██`;
    row += `${E}0m  `;
    for (let i = 0; i < 8; i++) row += `${E}${90 + i}m██`;
    line(row + `${E}0m  ${E}2m16 colors`);
    // The 6x6x6 cube and the grays.
    for (let g = 0; g < 6; g += 3) {
        row = ' ';
        for (let r = 0; r < 6; r++) for (let b = 0; b < 6; b++) row += `${E}48;5;${16 + r * 36 + g * 6 + b}m `;
        line(row + `${E}0m  ${E}2m${g === 0 ? '256 colors' : ''}`);
    }
    row = ' ';
    for (let i = 232; i < 256; i++) row += `${E}48;5;${i}m  `;
    line(row);
    // A true color sweep through the hues.
    row = ' ';
    for (let x = 0; x < width; x++) {
        const h = (x / width) * 6;
        const f = h - Math.floor(h);
        const [r, g, b] = [[1, f, 0], [1 - f, 1, 0], [0, 1, f], [0, 1 - f, 1], [f, 0, 1], [1, 0, 1 - f]][Math.floor(h) % 6].map((v) => Math.round(v * 255));
        row += `${E}48;2;${r};${g};${b}m `;
    }
    line(row + `${E}0m`);
    row = ' ';
    for (let x = 0; x < width; x++) {
        const v = Math.round((x / width) * 255);
        row += `${E}48;2;${v};${Math.round(v * 0.6)};${255 - v}m `;
    }
    line(row + `${E}0m  ${E}2m24-bit`);
    line();
    line(` ${E}1mBold${E}0m  ${E}2mDim${E}0m  ${E}3mItalic${E}0m  ${E}4mUnderline${E}0m  ${E}4:2mDouble${E}0m  ${E}4:3;58;5;203mCurly${E}0m  ${E}4:4mDotted${E}0m  ${E}9mStrike${E}0m  ${E}53mOverline${E}0m  ${E}7mReverse${E}0m`);
    line(` \x1b]8;;https://github.com/R1ck404/rsui\x1b\\A link a program made\x1b]8;;\x1b\\  ${E}2m(Ctrl+click it)`);
    line();
    line(` Ligatures  => != === >= <= -> <- |> :: /* */ www`);
    line(` Wide       日本語のテキスト 한국어 中文  emoji 🚀 🎉 👩‍💻 ❤️`);
    line(` Accents    é ñ ü ø å — “quotes” …  combining é ä`);
    line(` Symbols    ✓ ✗ ★ ☆ ◆ ● ○ ▲ ▼ ← → ↑ ↓ ⏎ ⌘ ⌥ ⇧`);
    line();
    line(` ┌──────────┬──────────┐  ╔══════════╗  ╭──────────╮  ▁▂▃▄▅▆▇█`);
    line(` │ boxes    │ join up  │  ║ double   ║  │ rounded  │  ░░▒▒▓▓██`);
    line(` ├──────────┼──────────┤  ╚══════════╝  ╰──────────╯  ▏▎▍▌▋▊▉█`);
    line(` └──────────┴──────────┘  ━━━━ ┃ ┏┓┗┛   ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`);
    return out.join('');
}
