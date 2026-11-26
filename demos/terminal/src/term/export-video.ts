import { spawn } from 'child_process';
import * as fs from 'fs';
import { VIDEO_FPS } from './recording-edits';

type VideoFrame =
    | { format: 'bgra'; width: number; height: number; data: Uint8Array }
    | { format: 'png'; data: Uint8Array };

/** Encode frames rendered by the app. No video frames are saved in the recording. */
export async function exportVideo(file: string, start: number, end: number, frame: (at: number, progress: number) => Promise<VideoFrame>) {
    const count = Math.max(1, Math.ceil((end - start) * VIDEO_FPS / 1000));
    const first = await frame(start, 1 / count);
    const source = first.format === 'bgra'
        ? ['-f', 'rawvideo', '-pixel_format', 'bgra', '-video_size', `${first.width}x${first.height}`]
        : ['-f', 'image2pipe', '-c:v', 'png'];
    const temporary = `${file}.${process.pid}-${Math.random().toString(36).slice(2)}.part.mp4`;
    const encoder = spawn('ffmpeg', [
        '-hide_banner', '-loglevel', 'error', '-y', ...source, '-framerate', String(VIDEO_FPS), '-i', 'pipe:0',
        '-vf', 'pad=ceil(iw/2)*2:ceil(ih/2)*2',
        '-c:v', 'libx264', '-preset', 'veryfast', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', temporary,
    ]);
    let stderr = '';
    let failure: Error | undefined;
    let launchFailure: Error | undefined;
    const closed = new Promise<void>((resolve) => {
        encoder.on('error', (error) => {
            launchFailure = error;
            failure = error;
            resolve();
        });
        encoder.on('close', (code) => {
            if (code !== 0) failure ??= new Error(stderr.trim() || 'FFmpeg stopped before the video was finished.');
            resolve();
        });
    });
    encoder.stderr?.on('data', (chunk: Uint8Array) => { stderr = (stderr + Buffer.from(chunk).toString('utf8')).slice(-4096); });
    encoder.stdin?.on('error', (error: Error) => { failure ??= error; });
    try {
        if (!encoder.stdin) {
            await closed;
            throw failure ?? new Error('Could not start FFmpeg.');
        }
        for (let i = 0; i < count; i++) {
            if (failure) throw failure;
            const next = i === 0 ? first : await frame(start + i * 1000 / VIDEO_FPS, (i + 1) / count);
            if (failure) throw failure;
            if (next.format !== first.format) throw new Error('The video capture format changed during export.');
            if (first.format === 'bgra' && next.format === 'bgra' &&
                (next.width !== first.width || next.height !== first.height || next.data.byteLength !== first.width * first.height * 4)) {
                throw new Error('The terminal preview changed pixel size during export.');
            }
            await new Promise<void>((resolve, reject) => {
                encoder.stdin.write(next.data, undefined, (error) => error ? reject(error) : resolve());
            });
        }
        encoder.stdin.end();
        await closed;
        if (failure) throw failure;
        await fs.promises.rename(temporary, file);
    } catch (error) {
        encoder.kill();
        await closed;
        await fs.promises.unlink(temporary).catch(() => {});
        const cause = launchFailure ?? error;
        const reason = stderr.trim() || (cause instanceof Error ? cause.message : String(cause));
        throw new Error(launchFailure && /ENOENT|executable not found|cannot find the file/i.test(launchFailure.message)
            ? 'FFmpeg is required to export MP4. Install it and make it available on PATH.'
            : reason);
    }
}
