import { defineConfig } from 'rsui/config';

export default defineConfig({
    id: 'dev.rsui.nativelook',
    name: 'Native look',
    // What `native` draws in: Material on Android and Fluent on Windows. 'material' or 'fluent' would draw that everywhere.
    theme: 'system',
});
