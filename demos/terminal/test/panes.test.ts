import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    offsetPanes, paneDropAt, paneHalf, paneRects, removePane, restorePaneLayout, splitPaneLayout, stackedPanes,
} from '../src/strip/panes.ts';

test('docking a stack splits the hovered pane and gives the incoming stack the previewed half', () => {
    const layout = stackedPanes(2);
    const target = paneRects(layout)[1].rect;
    assert.equal(paneDropAt(target, 0.9, 0.75), 'right');
    const docked = splitPaneLayout(layout, 1, offsetPanes(stackedPanes(2), 2), 'right');
    const incoming = paneHalf(target, 'right');
    assert.deepEqual(paneRects(docked), [
        { index: 0, rect: { x: 0, y: 0, w: 1, h: 0.5 } },
        { index: 1, rect: { x: 0, y: 0.5, w: 0.5, h: 0.5 } },
        { index: 2, rect: { ...incoming, h: 0.25 } },
        { index: 3, rect: { ...incoming, y: 0.75, h: 0.25 } },
    ]);
    assert.equal(paneDropAt(target, 0.9, 0.25), undefined);
    assert.deepEqual(restorePaneLayout(JSON.parse(JSON.stringify(docked)), 4), docked);
    assert.deepEqual(removePane(docked, 2), { axis: 'y', children: [0, { axis: 'x', children: [1, 2] }] });
    assert.equal(restorePaneLayout({ axis: 'x', children: [0, 0] }, 2), undefined);
});
