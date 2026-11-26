import { test } from 'node:test';
import assert from 'node:assert/strict';
import { columnWidth, GAP, overviewPlaces, stripPlaces } from '../src/strip/layout.ts';

test('a wide column leaves the neighbors peeking at both sides', () => {
    const w = columnWidth('wide', 1200, 44);
    assert.equal(w, 1200 - 2 * (44 + GAP));
    const places = stripPlaces([w, w, w], 1, 1200, 700);
    // The open one is in the middle, with `peek` pixels of each neighbor showing.
    assert.equal(places[1].x, 44 + GAP);
    assert.equal(places[0].x + places[0].w, GAP * 0 + 44);
    assert.equal(places[2].x, 1200 - 44);
});

test('the strip does not scroll past its ends', () => {
    const w = columnWidth('wide', 1000, 40);
    const first = stripPlaces([w, w, w], 0, 1000, 600);
    assert.equal(first[0].x, GAP);
    const last = stripPlaces([w, w, w], 2, 1000, 600);
    assert.equal(last[2].x + last[2].w, 1000 - GAP);
});

test('a few narrow columns sit together in the middle', () => {
    const w = columnWidth('third', 1200, 44);
    const places = stripPlaces([w], 0, 1200, 600);
    assert.ok(Math.abs(places[0].x + places[0].w / 2 - 600) <= GAP);
});

test('the overview fits every column in the window at one scale', () => {
    const w = 900;
    const places = overviewPlaces([w, w, w, w, w], 1300, 760);
    assert.equal(places.length, 5);
    const scale = places[0].scale;
    assert.ok(scale < 0.6 && scale > 0.2, String(scale));
    for (const p of places) {
        assert.equal(p.scale, scale);
        assert.ok(p.x >= 0 && p.x + p.w * scale <= 1300, `${p.x} ${p.w * scale}`);
        assert.ok(p.y >= 0 && p.y + p.h * scale <= 760);
    }
});
