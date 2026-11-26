// What `import ... from 'rsui/desktop'` gives: what a program can ask of the desktop, apart from what's on its windows, as one
// object for each part. Each part is only loaded the first time it's used, so a program pays for what it uses and nothing else.
// With `--do` and `--agent` none of it really happens, and it's said instead, and there are steps for what comes back.

const parts = {
    shell: 'rsui/desktop/basics',
    clipboard: 'rsui/desktop/basics',
    dialogs: 'rsui/desktop/basics',
    notifications: 'rsui/desktop/basics',
    tray: 'rsui/desktop/basics',
    shortcuts: 'rsui/desktop/basics',
    displays: 'rsui/desktop/basics',
    power: 'rsui/desktop/basics',
    sound: 'rsui/desktop/sound',
    app: 'rsui/desktop/app',
    storage: 'rsui/desktop/storage',
    updates: 'rsui/desktop/updates',
    secrets: 'rsui/desktop/more',
    taskbar: 'rsui/desktop/more',
    protocols: 'rsui/desktop/more',
    fileAssociations: 'rsui/desktop/more',
    system: 'rsui/desktop/more',
    network: 'rsui/desktop/more',
    drag: 'rsui/desktop/more',
    menus: 'rsui/desktop/more',
    files: 'rsui/desktop/pictures',
    capture: 'rsui/desktop/pictures',
    audio: 'rsui/desktop/media',
    media: 'rsui/desktop/media',
    video: 'rsui/desktop/media',
    pty: 'rsui/desktop/pty',
};

// As an ES module's exports, so that `import { clipboard } from 'rsui/desktop'` asks for `clipboard` alone, rather than every part
// being copied out, and so loaded, the moment it's imported.
Object.defineProperty(module.exports, '__esModule', { value: true });

for (const [name, from] of Object.entries(parts)) {
    Object.defineProperty(module.exports, name, {
        enumerable: true,
        configurable: true,
        get() {
            const value = require(from)[name];
            Object.defineProperty(module.exports, name, { value, enumerable: true, configurable: true, writable: false });
            return value;
        },
    });
}
