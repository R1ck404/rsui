// The shells a column can run, and the few lines each is given so it says where its prompts, commands and output are (OSC 133)
// and which folder it's in. Without them everything else still works; the marks in the gutter, jumping between prompts, copying
// a command's output and the finished-command notice need them.

export interface Profile {
    id: string;
    name: string;
    /** The command line, with `{script}` where the path of the integration script goes. */
    command: string;
    /** What goes in that script, if the shell has one. */
    script?: { name: string; text: string };
    env?: Record<string, string>;
    /** Set for what's started from here, as a profile the user made. */
    custom?: boolean;
    source?: 'ssh' | 'wsl';
    host?: string;
}

export function sshConfigEntries(config: string): { hosts: string[]; includes: string[] } {
    const hosts = new Set<string>();
    const includes = new Set<string>();
    for (const line of config.split(/\r?\n/)) {
        const entry = /^\s*(Host|Include)(?:\s*=\s*|\s+)(.*)/i.exec(line);
        if (!entry) continue;
        for (const token of entry[2].match(/"[^"]*"|#[^\n]*|[^\s#]+/g) ?? []) {
            if (token.startsWith('#')) break;
            const value = token.replace(/^"|"$/g, '');
            if (entry[1].toLowerCase() === 'include') includes.add(value);
            else if (/^[a-z0-9_][a-z0-9_.-]*$/i.test(value)) hosts.add(value);
        }
    }
    return { hosts: [...hosts], includes: [...includes] };
}

export function wslDistributions(output: Uint8Array): string[] {
    const bytes = output[0] === 0xff && output[1] === 0xfe ? output.subarray(2) : output;
    const utf16 = bytes.includes(0) && bytes.length % 2 === 0;
    let decoded: string;
    if (utf16) {
        const parts = [];
        for (let i = 0; i < bytes.length; i += 2) parts.push(String.fromCharCode(bytes[i] | (bytes[i + 1] << 8)));
        decoded = parts.join('');
    } else decoded = Buffer.from(bytes).toString('utf8');
    return [...new Set(decoded.split(/\r?\n/).map((name) => name.trim()).filter((name) => /^[\p{L}\p{N}_. -]{1,128}$/u.test(name)))];
}

const POWERSHELL_SCRIPT = String.raw`
$global:__rsuiStarted = $false
$global:__rsuiPrompt = $function:prompt
$global:__rsuiPromptConfig = $null
if ($env:RSUI_TERMINAL_PROMPT) {
    try { $global:__rsuiPromptConfig = ConvertFrom-Json $env:RSUI_TERMINAL_PROMPT -ErrorAction Stop } catch {}
}
function global:__rsuiStyledPrompt($loc, $exit) {
    $config = $global:__rsuiPromptConfig
    $e = [char]27
    $text = ''
    $previous = ''
    foreach ($segment in $config.segments) {
        $label = switch ($segment.id) {
            'shell' { 'PS' }
            'folder' {
                $cwd = $loc.Path
                if ($config.folder -eq 'name') {
                    $name = Split-Path -Leaf -Path $cwd
                    if ($name) { $cwd = $name }
                } elseif ($config.folder -eq 'home' -and $HOME) {
                    if ($cwd -eq $HOME) { $cwd = '~' }
                    elseif ($cwd.StartsWith($HOME + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { $cwd = '~' + $cwd.Substring($HOME.Length) }
                }
                $cwd
            }
            'git' {
                if ($loc.Provider.Name -eq 'FileSystem') {
                    $git = Get-Command git -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
                    if ($git) {
                        $branch = $null
                        try { $branch = & $git.Source symbolic-ref --quiet --short HEAD 2>$null } catch {}
                        if ($branch) { 'git:' + $branch }
                    }
                }
            }
            'status' { if ($exit -ne 0) { 'exit ' + $exit } }
            'time' { [DateTime]::Now.ToString('HH:mm') }
            'user' { $env:USERNAME }
        }
        if (-not $label) { continue }
        $label = ([string]$label) -replace '\p{Cc}', '?'
        if ($config.style -eq 'powerline') {
            if ($previous -and $config.edge) { $text += $segment.bg + $previous + $config.edge }
            $text += $segment.bg + $segment.fg + ' ' + $label + ' '
            $previous = $segment.edgeColor
        } else {
            if ($text) { $text += ' ' }
            $text += $segment.fg + $label + "$e[0m"
        }
    }
    if (-not $text) { return '> ' }
    if ($previous -and $config.edge) { $text += "$e[49m" + $previous + $config.edge }
    $text + "$e[0m" + $config.suffix
}
function global:prompt {
    $ok = $?
    $code = $global:LASTEXITCODE
    $exit = 0
    if (-not $ok) { if ($code) { $exit = $code } else { $exit = 1 } }
    $e = [char]27
    $bel = [char]7
    $out = ''
    if ($global:__rsuiStarted) { $out += "$e]133;D;$exit$bel" }
    $global:__rsuiStarted = $true
    $loc = $executionContext.SessionState.Path.CurrentLocation
    if ($loc.Provider.Name -eq 'FileSystem') { $out += "$e]9;9;$($loc.ProviderPath)$bel" }
    $out += "$e]133;A$bel"
    $global:LASTEXITCODE = $code
    if ($global:__rsuiPromptConfig -and $NestedPromptLevel -eq 0) {
        try { $text = __rsuiStyledPrompt $loc $exit }
        catch { $text = & $global:__rsuiPrompt }
        finally { $global:LASTEXITCODE = $code }
    } else {
        $text = & $global:__rsuiPrompt
    }
    $out + $text + "$e]133;B$bel"
}
try {
    Set-PSReadLineKeyHandler -Key Enter -ScriptBlock {
        [Console]::Write("$([char]27)]133;C$([char]7)")
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
    }
} catch {}
if (-not $env:RSUI_TERMINAL_RESTORED) { Clear-Host }
`;

const BASH_SCRIPT = `
[ -f ~/.bashrc ] && . ~/.bashrc
__rsui_prompt() {
    local c=$?
    printf '\\e]133;D;%s\\a\\e]133;A\\a' "$c"
    if command -v cygpath >/dev/null 2>&1; then printf '\\e]9;9;%s\\a' "$(cygpath -w "$PWD")"; else printf '\\e]7;file://%s%s\\a' "$HOSTNAME" "$PWD"; fi
}
PROMPT_COMMAND="__rsui_prompt\${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
PS1="$PS1\\[\\e]133;B\\a\\]"
PS0='\\[\\e]133;C\\a\\]'
`;

const CMD_PROMPT = '$E]133;A$E\\$E]9;9;$P$E\\$P$G$E]133;B$E\\';

/** The shells that are on this machine. */
export function availableProfiles(exists: (path: string) => boolean): Profile[] {
    const system = (process.env.SystemRoot ?? 'C:\\Windows') + '\\System32';
    const list: Profile[] = [];
    const pwsh = ['C:\\Program Files\\PowerShell\\7\\pwsh.exe', 'C:\\Program Files\\PowerShell\\7-preview\\pwsh.exe'].find(exists);
    if (pwsh) {
        list.push({
            id: 'pwsh', name: 'PowerShell 7', command: `"${pwsh}" -NoLogo -NoExit -ExecutionPolicy Bypass -File "{script}"`,
            script: { name: 'integration.ps1', text: POWERSHELL_SCRIPT },
        });
    }
    list.push({
        id: 'powershell', name: 'Windows PowerShell', command: `"${system}\\WindowsPowerShell\\v1.0\\powershell.exe" -NoLogo -NoExit -ExecutionPolicy Bypass -File "{script}"`,
        script: { name: 'integration.ps1', text: POWERSHELL_SCRIPT },
    });
    list.push({ id: 'cmd', name: 'Command Prompt', command: `"${system}\\cmd.exe"`, env: { PROMPT: CMD_PROMPT } });
    const bash = ['C:\\Program Files\\Git\\bin\\bash.exe', 'C:\\Program Files (x86)\\Git\\bin\\bash.exe'].find(exists);
    if (bash) {
        list.push({
            id: 'gitbash', name: 'Git Bash', command: `"${bash}" --rcfile "{script}" -i`,
            script: { name: 'integration.bash', text: BASH_SCRIPT },
            env: { CHERE_INVOKING: '1' },
        });
    }
    if (exists(`${system}\\wsl.exe`)) list.push({ id: 'wsl', name: 'WSL', command: `"${system}\\wsl.exe"`, source: 'wsl' });
    return list;
}
