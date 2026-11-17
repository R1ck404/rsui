
// More of what a window's controls do: the buttons under its picture in the taskbar, and what the system draws behind it. This
// is read as the end of `layer.js`, in the same module, so `WindowControls` and `host` are that file's.

/// What each window's buttons under its picture in the taskbar do, by the window, to forget when they're changed.
const thumbnailActions = new Map();
const backdrops = ['mica', 'acrylic', 'tabbed', 'none'];

Object.defineProperties(WindowControls.prototype, {
    /// Puts up to seven buttons under its picture in the taskbar, which shows when the pointer's over its button, in place of
    /// those that were there: `{ id, icon, tooltip, disabled, onClick }` each, with `icon` a file or a picture's bytes. `[]`
    /// takes them away. While an app's being checked, the `thumb ID` step clicks one.
    setThumbnailButtons: {
        value: function setThumbnailButtons(buttons) {
            const list = Array.isArray(buttons) ? buttons : [];
            if (list.length > 7) {
                throw new RangeError('the taskbar has room for 7 buttons under a window\'s picture, at most');
            }
            const { action, forget, pictureBytes } = require('rsui/desktop/core');
            forget(thumbnailActions.get(this.id) ?? []);
            const used = [];
            const spec = list.map((button, at) => {
                const id = action(button.onClick);
                if (id !== 0) {
                    used.push(id);
                }
                return {
                    name: String(button.id ?? at),
                    action: id,
                    icon: pictureBytes(button.icon),
                    tooltip: String(button.tooltip ?? ''),
                    disabled: button.disabled === true,
                };
            });
            thumbnailActions.set(this.id, used);
            host.thumbnailButtons(this.id, spec);
        },
        writable: true,
        configurable: true,
    },
    /// Has the system draw this behind the window, as Windows 11's own programs have: `mica`, `acrylic`, `tabbed`, or `none`
    /// for its usual background. The title bar goes dark or light with the system's colors. rsui draws all of what's inside the
    /// window itself, so for now it's the title bar the backdrop shows in.
    setBackdrop: {
        value: function setBackdrop(kind = 'mica') {
            const wanted = String(kind);
            if (!backdrops.includes(wanted)) {
                throw new TypeError(`a backdrop is ${backdrops.join(', ')}, not ${wanted}`);
            }
            host.changeWindow(this.id, 'backdrop', wanted);
        },
        writable: true,
        configurable: true,
    },
});
