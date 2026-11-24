import { defineConfig } from 'rsui/config';

// Who the app is and how it's packed: see `rsui/config` in rsui.d.ts. `rsui-tsx config` prints what this comes to.
export default defineConfig({
    id: 'dev.rsui.mobilehello',
    name: 'Hello',
    version: '0.1.0',
    android: { orientation: 'portrait' },
});
