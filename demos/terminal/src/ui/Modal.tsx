import { useEffect, useLayoutEffect, useRef } from 'react';
import type { Child } from 'rsui';
import { withAlpha } from '../themes';

type ModalPanel = HTMLDivElement & { querySelectorAll(selector: string): ArrayLike<HTMLElement> };

const focusable = 'button, input, select, textarea, a[href], [tabindex]';

export function Modal({ label, children, className, style, backdrop = 0.4, inset = '', onClose, onEscape = onClose }: {
    label: string;
    children: Child;
    className: string;
    style: PageStyle;
    backdrop?: number;
    inset?: string;
    onClose: () => void;
    onEscape?: () => void;
}) {
    const panel = useRef<ModalPanel>(null);
    useLayoutEffect(() => {
        const element = panel.current;
        if (!element) return;
        const previous = document.activeElement;
        const initial = element.querySelectorAll('[data-dialog-focus]')[0] ?? element;
        initial.focus();
        return () => previous?.focus();
    }, []);

    useEffect(() => {
        const onKeyDown = (event: KeyboardEvent) => {
            const element = panel.current;
            if (!element || event.defaultPrevented || event.isComposing) return;
            if (event.key === 'Escape') {
                event.preventDefault();
                onEscape();
                return;
            }
            if (event.key !== 'Tab' || event.ctrlKey || event.altKey || event.metaKey) return;
            event.preventDefault();
            const stops = Array.from(element.querySelectorAll(focusable)).filter((stop) =>
                !stop.hasAttribute('disabled') && stop.getAttribute('tabindex') !== '-1'
                && !stop.closest('[hidden], [inert]') && stop.offsetWidth > 0 && stop.offsetHeight > 0);
            if (!stops.length) {
                element.focus();
                return;
            }
            const index = stops.indexOf(document.activeElement as HTMLElement);
            const next = event.shiftKey ? (index <= 0 ? stops.length - 1 : index - 1) : (index + 1) % stops.length;
            stops[next].focus();
        };
        window.addEventListener('keydown', onKeyDown);
        return () => window.removeEventListener('keydown', onKeyDown);
    }, [onEscape]);

    return (
        <div className={`absolute inset-0 z-30 flex items-center justify-center ${inset}`}
            style={{ background: withAlpha(0x000000, backdrop) }} onPointerDown={onClose}>
            <div ref={(element) => { panel.current = element as ModalPanel | null; }} role="dialog" aria-modal="true"
                aria-label={label} tabIndex={-1} className={className} style={style}
                onPointerDown={(event) => event.stopPropagation()}>
                {children}
            </div>
        </div>
    );
}
