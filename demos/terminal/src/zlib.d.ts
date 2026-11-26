declare module 'zlib' {
    export function crc32(data: Uint8Array): number;
    export function deflateSync(data: Uint8Array, options?: { level?: number }): Buffer;
    export function inflateSync(data: Uint8Array, options?: { maxOutputLength?: number }): Buffer;
    export function gzip(data: Uint8Array, options: { level?: number }, done: (error: Error | null, result: Buffer) => void): void;
    export function gunzipSync(data: Uint8Array, options?: { maxOutputLength?: number }): Buffer;
}
