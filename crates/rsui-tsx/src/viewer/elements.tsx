// The Elements pane: what's in an app's window, as a tree, and what's chosen in it, with its box and where it came from.
import { useState, useEffect, useCallback } from 'rsui';
import { MONO, ROW, TREE_EVERY, UI } from './theme';
import type { Ask, El, Made, Theme } from './theme';
import { Across, Empty, Icon, Splitter, TabStrip } from './ui';

/// What a tag's written as in JSX, from what the tree calls its role.
export function tagOf(element: El): string {
    switch (element.tag) {
        case 'text_input':
            return 'input';
        case 'list_item':
            return 'item';
        case 'menu_item':
            return 'menuitem';
        case 'group':
            return element.props.direction === 'row' ? 'row' : element.props.direction === 'column' ? 'column' : 'group';
        default:
            return element.tag;
    }
}

/// A prop's name as JSX writes it.
export function camel(name: string): string {
    return name.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase());
}

export function shownValue(value: any): string {
    return typeof value === 'string' ? JSON.stringify(value) : `{${String(value)}}`;
}

export const SAID_INLINE = 4;

/// An element's opening tag, with its key and its first few props, as a browser's inspector writes one.
export function OpeningTag({ element, theme, closed }: { element: El; theme: Theme; closed: string | null }) {
    const tag = tagOf(element);
    const props = Object.entries(element.props).filter(([name]) => name !== 'direction' && !name.startsWith('scroll_') && name !== 'first_visible');
    return (
        <text fontSize={12} fontFamily={MONO} noWrap>
            <span color={theme.tag}>{`<${tag}`}</span>
            {element.key && !/^\d+$/.test(element.key) ? (
                <span>
                    <span color={theme.attr}>{' key'}</span>
                    <span color={theme.dim}>=</span>
                    <span color={theme.value}>{JSON.stringify(element.key)}</span>
                </span>
            ) : null}
            {props.slice(0, SAID_INLINE).map(([name, value]) => (
                <span key={`prop-${name}`}>
                    <span color={theme.attr}>{` ${camel(name)}`}</span>
                    <span color={theme.dim}>=</span>
                    <span color={theme.value}>{shownValue(value)}</span>
                </span>
            ))}
            {props.length > SAID_INLINE ? <span color={theme.dim}>{' …'}</span> : null}
            <span color={theme.tag}>{'>'}</span>
            {closed !== null ? <span color={theme.text}>{closed}</span> : null}
            {closed !== null ? <span color={theme.tag}>{`</${tag}>`}</span> : null}
        </text>
    );
}

/// One element's row in the tree.
export function TreeRow({ element, id, depth, has, opened, chosen, flip, choose, point, theme }: { element: El; id: string; depth: number; has: boolean; opened: boolean; chosen: boolean; flip: (id: string) => void; choose: (id: string) => void; point: (element: El | null) => void; theme: Theme }) {
    // Something with only a name in it is written on one line, as text is.
    const closed = !has ? element.name : opened ? null : '…';
    return (
        <row height={ROW} align="center" background={chosen ? theme.selected : 'transparent'} hover={chosen ? theme.selected : theme.hover} onMouseEnter={() => point(element)}>
            <group width={depth * 14 + 4} />
            {has ? (
                <button name={`${opened ? 'Close' : 'Open'} ${id}`} width={14} height={ROW} align="center" justify="center" onClick={() => flip(id)}>
                    <Icon name={opened ? 'down' : 'right'} color={theme.dim} size={10} />
                </button>
            ) : (
                <group width={14} />
            )}
            <button direction="row" justify="start" name={`Element ${id}`} flex={1} height={ROW} align="center" onClick={() => choose(id)} onDoubleClick={() => has && flip(id)}>
                <OpeningTag element={element} theme={theme} closed={closed} />
            </button>
        </row>
    );
}

/// The elements in the app's window as a tree, opened as far as it's been opened, with each row's own element filled in
/// in the app while the pointer's on it.
export function ElementTree({ root, open, flip, chosen, choose, point, theme }: { root: El; open: Set<string>; flip: (id: string) => void; chosen: string; choose: (id: string) => void; point: (element: El | null) => void; theme: Theme }) {
    const rows: any[] = [];
    const walk = (element: El, depth: number) => {
        const id = element.path.join('.');
        const has = element.children.length > 0;
        const opened = has && open.has(id);
        rows.push(<TreeRow key={`el-${id}`} element={element} id={id} depth={depth} has={has} opened={opened} chosen={chosen === id} flip={flip} choose={choose} point={point} theme={theme} />);
        if (opened) {
            for (const child of element.children) {
                walk(child, depth + 1);
            }
            if (element.more > 0) {
                rows.push(
                    <row key={`more-${id}`} height={ROW} align="center">
                        <group width={(depth + 1) * 14 + 18} />
                        <text fontSize={12} fontFamily={UI} color={theme.dim}>{`${element.more} more aren't shown`}</text>
                    </row>,
                );
            }
            rows.push(
                <row key={`close-${id}`} height={ROW} align="center" hover={theme.hover}>
                    <group width={depth * 14 + 18} />
                    <text fontSize={12} fontFamily={MONO} color={theme.tag}>{`</${tagOf(element)}>`}</text>
                </row>,
            );
        }
    };
    walk(root, 0);
    return (
        <scroll key="element-tree" flex={1} axis="both" paddingY={4} onMouseLeave={() => point(null)}>
            {rows}
        </scroll>
    );
}

/// The box a browser's inspector draws for an element: its margin, border and padding around what's inside, with how big
/// each is.
export function BoxModel({ element, theme }: { element: El; theme: Theme }) {
    const prop = (names: string[]) => {
        for (const name of names) {
            const value = element.props[name];
            if (typeof value === 'number') {
                return value;
            }
        }
        return 0;
    };
    const side = (kind: string, which: string, across: string) => prop([`${kind}_${which}`, `${kind}${which[0].toUpperCase()}${which.slice(1)}`, `${kind}_${across}`, kind]);
    const padding = { top: side('padding', 'top', 'y'), right: side('padding', 'right', 'x'), bottom: side('padding', 'bottom', 'y'), left: side('padding', 'left', 'x') };
    const margin = { top: side('margin', 'top', 'y'), right: side('margin', 'right', 'x'), bottom: side('margin', 'bottom', 'y'), left: side('margin', 'left', 'x') };
    const border = prop(['border_width', 'borderWidth']);
    const [, , width, height] = element.rect ?? [0, 0, 0, 0];
    const inner = `${Math.round(width - padding.left - padding.right - border * 2)} × ${Math.round(height - padding.top - padding.bottom - border * 2)}`;
    const number = (value: number) => (value === 0 ? '-' : String(Math.round(value * 10) / 10));
    const label = (text: string) => <text fontSize={10} fontFamily={UI} color={theme.box}>{text}</text>;
    const ring = (name: string, color: string, sides: { top: number; right: number; bottom: number; left: number }, inside: any) => (
        <column background={color} borderWidth={1} borderColor={theme.strong} paddingX={6} paddingY={2} align="center" minWidth={120}>
            <row width="100%">
                {label(name)}
                <group flex={1} />
            </row>
            {label(number(sides.top))}
            <row align="center" gap={6}>
                {label(number(sides.left))}
                {inside}
                {label(number(sides.right))}
            </row>
            {label(number(sides.bottom))}
        </column>
    );
    return (
        <column align="center" padding={12}>
            {ring(
                'margin',
                theme.margin,
                margin,
                ring(
                    'border',
                    theme.border,
                    { top: border, right: border, bottom: border, left: border },
                    ring(
                        'padding',
                        theme.padding,
                        padding,
                        <column background={theme.content} borderWidth={1} borderColor={theme.strong} paddingX={12} paddingY={6} align="center">
                            {label(element.rect ? inner : 'not laid out')}
                        </column>,
                    ),
                ),
            )}
            {element.rect ? (
                <text fontSize={11} fontFamily={MONO} color={theme.dim} paddingTop={8} selectable>
                    {`at ${Math.round(element.rect[0])}, ${Math.round(element.rect[1])}, ${Math.round(width)} × ${Math.round(height)}`}
                </text>
            ) : null}
        </column>
    );
}

/// What's known about the element chosen: its props, its box, the component that made it, and what it does.
export function ElementSidebar({ element, made, theme }: { element: El | null; made: Made | null; theme: Theme }) {
    const [tab, setTab] = useState('Props');
    if (!element) {
        return <Empty text="Choose an element to see its props, its box, and the component that made it." theme={theme} />;
    }
    const props = Object.entries(element.props);
    return (
        <column flex={1}>
            <TabStrip tabs={['Props', 'Layout', 'Component', 'Listeners']} value={tab} onChange={setTab} theme={theme} />
            {tab === 'Props' ? (
                <scroll key="props" flex={1} padding={8} gap={2}>
                    <text fontSize={12} fontFamily={MONO} color={theme.tag} paddingBottom={4}>{`${tagOf(element)} {`}</text>
                    {element.name ? (
                        <row key="prop-name" paddingLeft={16}>
                            <text fontSize={12} fontFamily={MONO} color={theme.attr}>name</text>
                            <text fontSize={12} fontFamily={MONO} color={theme.dim}>{': '}</text>
                            <text flex={1} fontSize={12} fontFamily={MONO} color={theme.str} selectable>{JSON.stringify(element.name)}</text>
                        </row>
                    ) : null}
                    {props.map(([name, value]) => (
                        <row key={`prop-${name}`} paddingLeft={16}>
                            <text fontSize={12} fontFamily={MONO} color={theme.attr}>{camel(name)}</text>
                            <text fontSize={12} fontFamily={MONO} color={theme.dim}>{': '}</text>
                            <text flex={1} fontSize={12} fontFamily={MONO} color={typeof value === 'string' ? theme.str : theme.num} selectable>
                                {typeof value === 'string' ? JSON.stringify(value) : String(value)}
                            </text>
                        </row>
                    ))}
                    <text fontSize={12} fontFamily={MONO} color={theme.tag}>{'}'}</text>
                    {props.length === 0 && !element.name ? <text fontSize={12} fontFamily={UI} color={theme.dim} paddingTop={6}>It has no props of its own.</text> : null}
                </scroll>
            ) : null}
            {tab === 'Layout' ? (
                <scroll key="layout" flex={1}>
                    <BoxModel element={element} theme={theme} />
                </scroll>
            ) : null}
            {tab === 'Component' ? (
                <scroll key="component" flex={1} padding={10} gap={8}>
                    {made === null ? <text fontSize={12} fontFamily={UI} color={theme.dim}>Finding out.</text> : null}
                    {made && made.component ? <text fontSize={14} fontFamily={MONO} color={theme.fn}>{`<${made.component}>`}</text> : null}
                    {made && made.around.length > 0 ? <text fontSize={12} fontFamily={UI} color={theme.dim} selectable>{`In ${made.around.join(' › ')}`}</text> : null}
                    {made && made.why ? (
                        <column gap={4}>
                            <text fontSize={11} fontFamily={UI} fontWeight="bold" color={theme.dim}>WHY IT RENDERED</text>
                            <text fontSize={12} fontFamily={UI} color={theme.text} selectable>{made.why}</text>
                        </column>
                    ) : null}
                    {made && made.hooks.length > 0 ? (
                        <column gap={2}>
                            <text fontSize={11} fontFamily={UI} fontWeight="bold" color={theme.dim}>HOOKS</text>
                            {made.hooks.map((hook, index) => (
                                <text key={`hook-${index}`} fontSize={12} fontFamily={MONO} color={theme.text} selectable>{hook}</text>
                            ))}
                        </column>
                    ) : null}
                    {made && !made.component && !made.why ? (
                        <text fontSize={12} fontFamily={UI} color={theme.dim}>What made it hasn't rendered since the app started keeping that. Use the app, and it shows here.</text>
                    ) : null}
                </scroll>
            ) : null}
            {tab === 'Listeners' ? (
                <scroll key="listeners" flex={1} padding={10} gap={4}>
                    {element.actions.length === 0 ? <text fontSize={12} fontFamily={UI} color={theme.dim}>Nothing here listens for anything.</text> : null}
                    {element.actions.map((action) => (
                        <row key={`action-${action}`} gap={6} align="center">
                            <Icon name="right" color={theme.dim} size={10} />
                            <text fontSize={12} fontFamily={MONO} color={theme.text}>{action}</text>
                        </row>
                    ))}
                </scroll>
            ) : null}
        </column>
    );
}

export function findElement(root: El | null, id: string): El | null {
    if (!root) {
        return null;
    }
    if (root.path.join('.') === id) {
        return root;
    }
    for (const child of root.children) {
        const found = findElement(child, id);
        if (found) {
            return found;
        }
    }
    return null;
}

export function Elements({ ask, theme, gone, picked }: { ask: Ask; theme: Theme; gone: boolean; picked?: { path: number[]; at: number } | null }) {
    const [root, setRoot] = useState<El | null>(null);
    const [cut, setCut] = useState(false);
    const [said, setSaid] = useState<string | null>(null);
    const [open, setOpen] = useState<Set<string>>(new Set(['', '0', '0.0']));
    const [chosen, setChosen] = useState('');
    const [made, setMade] = useState<Made | null>(null);
    const [side, setSide] = useState(360);
    useEffect(() => {
        let stopped = false;
        let timer: any = null;
        let last = '';
        const again = async () => {
            const answer = await ask({ ask: 'tree' });
            if (stopped) {
                return;
            }
            if (answer.error) {
                setSaid(answer.error);
            } else {
                // Only what's changed is shown again: the same tree keeps every row as it was.
                const now = JSON.stringify(answer.tree);
                if (now !== last) {
                    last = now;
                    setRoot(answer.tree);
                    setCut(answer.cut);
                }
                setSaid(null);
            }
            timer = setTimeout(again, TREE_EVERY);
        };
        if (!gone) {
            again();
        }
        return () => {
            stopped = true;
            clearTimeout(timer);
            ask({ ask: 'highlight' });
        };
    }, [gone]);
    // What was picked in the app is chosen, with what it's in opened to show it.
    useEffect(() => {
        if (!picked) {
            return;
        }
        const id = picked.path.join('.');
        setOpen((was: Set<string>) => {
            const next = new Set(was);
            next.add('');
            for (let depth = 1; depth < picked.path.length; depth += 1) {
                next.add(picked.path.slice(0, depth).join('.'));
            }
            return next;
        });
        setChosen(id);
    }, [picked?.at]);
    const element = findElement(root, chosen);
    useEffect(() => {
        setMade(null);
        if (element && !gone) {
            ask({ ask: 'element', path: element.path }).then((answer) => setMade(answer.error ? null : (answer as any)));
        }
    }, [chosen]);
    const point = useCallback(
        (at: El | null) => {
            if (!gone) {
                ask(at ? { ask: 'highlight', path: at.path, name: tagOf(at) } : { ask: 'highlight' });
            }
        },
        [gone],
    );
    const flip = useCallback((id: string) => {
        setOpen((was: Set<string>) => {
            const next = new Set(was);
            if (next.has(id)) {
                next.delete(id);
            } else {
                next.add(id);
            }
            return next;
        });
    }, []);
    // Where the chosen element is, as tags from the window in.
    const crumbs: El[] = [];
    let at: El | null = root;
    const steps = chosen === '' ? [] : chosen.split('.');
    if (at) {
        crumbs.push(at);
        for (const step of steps) {
            at = at?.children[Number(step)] ?? null;
            if (at) {
                crumbs.push(at);
            }
        }
    }
    return (
        <row flex={1}>
            <column flex={1}>
                {root ? (
                    <ElementTree root={root} open={open} flip={flip} chosen={chosen} choose={setChosen} point={point} theme={theme} />
                ) : (
                    <Empty text={said ?? (gone ? 'The app has stopped.' : 'Asking the app what its window has in it.')} theme={theme} />
                )}
                {cut ? <text fontSize={11} fontFamily={UI} color={theme.dim} paddingX={8}>The window has more than can be shown at once, so only the first part is.</text> : null}
                <Across theme={theme} />
                <row height={24} align="center" paddingX={4} gap={2} background={theme.bar}>
                    {crumbs.map((crumb, index) => (
                        <button key={`crumb-${index}`} name={`Crumb ${crumb.path.join('.')}`} onClick={() => setChosen(crumb.path.join('.'))} direction="row" paddingX={6} height={20} radius={3} align="center" hover={theme.hover} background={index === crumbs.length - 1 ? theme.chosen : 'transparent'}>
                            <text fontSize={11} fontFamily={MONO} color={index === crumbs.length - 1 ? theme.tag : theme.dim} noWrap>
                                {`${tagOf(crumb)}${crumb.key && !/^\d+$/.test(crumb.key) ? `#${crumb.key}` : ''}`}
                            </text>
                        </button>
                    ))}
                </row>
            </column>
            <Splitter width={side} setWidth={setSide} least={240} most={640} theme={theme} fromRight />
            <column width={side}>
                <ElementSidebar element={element} made={made} theme={theme} />
            </column>
        </row>
    );
}

// Console.
