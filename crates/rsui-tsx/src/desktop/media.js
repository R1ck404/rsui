// `audio` and `media` in `rsui/desktop`: the devices that play and record sound, and the keys on a keyboard for what's playing.

const { host, action, topic } = require('rsui/desktop/core');

const listed = (devices) => devices.map(({ id, name, kind, isDefault }) => `${kind} ${id} ${name} ${isDefault}`).join('\n');

let known = '';
const deviceChanges = topic((tell) => {
    known = listed(host.audioDevices());
    host.audioOnChange(action(() => {
        // A device of any kind being plugged in is told, and only a change to these is passed on.
        const now = host.audioDevices();
        if (listed(now) !== known) {
            known = listed(now);
            tell(now);
        }
    }));
});

const audio = {
    /// The devices that play sound and record it that are there now: `{ id, name, kind, isDefault }`, with `kind` `output` or
    /// `input`, and `isDefault` whether it's what the system uses for that.
    devices: () => host.audioDevices(),
    /// Runs with the devices when one's plugged in or taken out. What it gives back stops it.
    onChange: (run) => deviceChanges.on(run),
};

const keys = topic((tell) => host.mediaOnKey(action(tell)));

const media = {
    /// Runs with `play`, `pause`, `playPause`, `next`, `previous` or `stop` when one of those keys is pressed while one of this
    /// program's windows has the keyboard. They're not taken from other programs, which get them when they have it. While an app's
    /// being checked, the `media KEY` step presses one. What it gives back stops it.
    onKey: (run) => keys.on(run),
};

module.exports = { audio, media };
