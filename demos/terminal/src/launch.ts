// Turning a profile into a command line that starts the shell with its integration script, which is written to the temp folder.

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { execFile } from 'child_process';
import { availableProfiles, sshConfigEntries, wslDistributions } from './shells';
import type { Profile } from './shells';
import type { Settings } from './settings';
import { compilePrompt, DEFAULT_PROMPT } from './prompt';
import type { PromptSettings } from './prompt';

let profiles: Profile[] | null = null;
let wslProfiles: Profile[] = [];
let sshProfiles: Profile[] = [];
let discovery = 0;

async function sshHosts(folder: string): Promise<string[]> {
    const names = new Set<string>();
    const seen = new Set<string>();
    const visit = async (file: string, depth: number) => {
        const key = file.toLowerCase();
        if (depth > 8 || seen.size >= 64 || names.size >= 256 || seen.has(key)) return;
        seen.add(key);
        try {
            const stat = await fs.promises.stat(file);
            if (!stat.isFile() || stat.size > 256 * 1024) return;
            const { hosts, includes } = sshConfigEntries(await fs.promises.readFile(file, 'utf8'));
            for (const host of hosts.slice(0, 256 - names.size)) names.add(host);
            for (const include of includes.slice(0, 64)) {
                const target = path.resolve(folder, include.replace(/^~[\\/]/, os.homedir() + '/'));
                const directory = path.dirname(target);
                const pattern = path.basename(target);
                if (!/[?*]/.test(pattern)) await visit(target, depth + 1);
                else {
                    const regex = new RegExp('^' + pattern.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.') + '$', 'i');
                    const entries = await fs.promises.readdir(directory).catch(() => []);
                    for (const name of entries.filter((name) => regex.test(name)).sort().slice(0, 64)) await visit(path.join(directory, name), depth + 1);
                }
            }
        } catch { /* A missing or unreadable include doesn't affect the other profiles. */ }
    };
    await visit(path.join(folder, 'config'), 0);
    return [...names];
}

export async function discoverProfiles(settings: Settings) {
    const scan = ++discovery;
    const ssh = settings.discoverSsh ? sshHosts(path.join(os.homedir(), '.ssh')) : Promise.resolve([]);
    const base = settings.discoverWsl ? allProfiles(settings).find((profile) => profile.id === 'wsl') : undefined;
    const wsl = base ? new Promise<Profile[]>((resolve) => {
        const exe = base.command.slice(1, -1);
        execFile(exe, ['--list', '--quiet'], { encoding: 'buffer', timeout: 2500 }, (error, output) => {
            const bytes: unknown = output;
            resolve(error || !(bytes instanceof Uint8Array) ? [] : wslDistributions(bytes).map((name) => ({
                id: 'wsl:' + name, name: 'WSL · ' + name, command: `${base.command} --distribution "${name}"`, source: 'wsl',
            })));
        });
    }) : Promise.resolve([]);
    const [hosts, distributions] = await Promise.all([ssh, wsl]);
    if (scan !== discovery) return;
    sshProfiles = hosts.map((host) => ({ id: 'ssh:' + host, name: 'SSH · ' + host, command: `ssh.exe "${host}"`, host, source: 'ssh' }));
    wslProfiles = distributions;
}

export function allProfiles(settings: Settings): Profile[] {
    profiles ??= availableProfiles((file) => {
        try {
            return fs.existsSync(file);
        } catch {
            return false;
        }
    });
    const list = [...profiles];
    if (settings.discoverSsh) list.push(...sshProfiles);
    if (settings.discoverWsl) list.push(...wslProfiles);
    if (settings.customCommand.trim() !== '') {
        list.push({ id: 'custom', name: 'Custom', command: settings.customCommand.trim(), custom: true });
    }
    return list;
}

export function profileById(settings: Settings, id: string): Profile {
    const list = allProfiles(settings);
    return list.find((profile) => profile.id === id) ?? list[0];
}

/** Writes a profile's script, and gives the command line to run and what to add to the environment. */
export function launchFor(profile: Profile, prompt: PromptSettings = DEFAULT_PROMPT, cwd?: string): { command: string; env: Record<string, string> } {
    let command = profile.command;
    if (profile.source === 'wsl' && cwd?.startsWith('/') && !/[\x00-\x1f"\\]/.test(cwd)) command += ` --cd "${cwd}"`;
    if (profile.script) {
        const folder = path.join(os.tmpdir(), 'rsui-terminal');
        try {
            fs.mkdirSync(folder, { recursive: true });
            const file = path.join(folder, profile.script.name);
            if (!fs.existsSync(file) || fs.readFileSync(file, 'utf8') !== profile.script.text) fs.writeFileSync(file, profile.script.text);
            command = command.replace('{script}', file.replace(/\\/g, profile.id === 'gitbash' ? '/' : '\\'));
        } catch {
            // Without the script the shell runs as it is.
            command = command.replace(/ -File "\{script\}"| --rcfile "\{script\}"/, '');
        }
    }
    const env = { ...profile.env };
    if (profile.id === 'powershell' || profile.id === 'pwsh') {
        env.RSUI_TERMINAL_PROMPT = prompt.style === 'shell' ? '' : JSON.stringify(compilePrompt(prompt));
    }
    return { command, env };
}
