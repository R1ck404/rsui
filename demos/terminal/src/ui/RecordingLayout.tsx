import { createContext, useLayoutEffect, useSyncExternalStore } from 'react';
import type { Child } from 'rsui';
import { createStore } from '../settings';
import { tokens } from '../themes';
import type { Scheme } from '../themes';

export const createRecordingSidebar = () => createStore<{ content: Child }>({ content: null });
type Sidebar = ReturnType<typeof createRecordingSidebar>;
export const RecordingSidebarContext = createContext<Sidebar | null>(null);

export function RecordingSidebar({ sidebar }: { sidebar: Sidebar }) {
    const { content } = useSyncExternalStore(sidebar.subscribe, sidebar.get);
    return <div className="min-h-0 flex-1 overflow-y-auto px-2">{content}</div>;
}

export interface RecordingNavigation {
    inline: boolean;
    sidebar: Sidebar | null;
    library: Child;
}

export function RecordingLayout({ nav, sidebar, scheme, children }: {
    nav: RecordingNavigation;
    sidebar?: Child;
    scheme: Scheme;
    children: Child;
}) {
    const content = sidebar ?? nav.library;
    const t = tokens(scheme);
    const outside = nav.inline ? null : nav.sidebar;
    useLayoutEffect(() => {
        if (outside) return () => outside.set({ content: null });
    }, [outside]);
    useLayoutEffect(() => { outside?.set({ content }); }, [outside, content]);
    return (
        <div className="flex size-full min-h-0 overflow-hidden" style={{ color: t.text }}>
            {nav.inline ? (
                <aside className={`shrink-0 overflow-y-auto px-2 py-3 ${sidebar ? 'w-[156px] max-w-[28%]' : 'w-[216px] max-w-[36%]'}`}>
                    {content}
                </aside>
            ) : null}
            <div className="relative flex min-w-0 flex-1 flex-col overflow-hidden">{children}</div>
        </div>
    );
}
