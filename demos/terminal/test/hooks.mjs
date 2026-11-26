// Lets Node run the app's TypeScript as the engine does, where an import names no extension.
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

export async function resolve(specifier, context, next) {
    const named = specifier.lastIndexOf('.') > specifier.lastIndexOf('/');
    if (specifier.startsWith('.') && !named && context.parentURL) {
        for (const ext of ['.ts', '.tsx', '/index.ts']) {
            const url = new URL(specifier + ext, context.parentURL);
            if (existsSync(fileURLToPath(url))) return next(url.href, context);
        }
    }
    return next(specifier, context);
}
