# Runs the native-look bench headless, sandboxed, for native and hand-styled buttons, and says how long each step took and
# how much memory there was. -Exe is the rsui-tsx to run, -Count how many buttons, -Theme the look, -Out where output goes.
param(
    [string]$Exe = "$env:CARGO_TARGET_DIR\release\rsui-tsx.exe",
    [int]$Count = 1000,
    [string]$Theme = 'material',
    [string]$Out = "$env:TEMP\native-look-bench",
    [int]$Runs = 3
)
(Get-Process -Id $PID).PriorityClass = 'BelowNormal'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
New-Item -ItemType Directory -Force $Out | Out-Null
foreach ($run in 1..$Runs) {
    foreach ($mode in @('native', 'plain')) {
        $sandbox = Join-Path $Out "sandbox-$mode"
        New-Item -ItemType Directory -Force "$sandbox\roaming", "$sandbox\local" | Out-Null
        $env:RSUI_SANDBOX = $sandbox
        $env:APPDATA = "$sandbox\roaming"
        $env:LOCALAPPDATA = "$sandbox\local"
        $env:RSUI_OFFLINE = '1'
        $env:RSUI_STEP_TIMES = '1'
        $env:RSUI_DO_SETTLE_MS = '0'
        $env:RSUI_THEME = $Theme
        $env:RSUI_COLOR_SCHEME = 'light'
        $env:RSUI_ACCENT = 'default'
        $env:NATIVE_LOOK_N = "$Count"
        $env:NATIVE_LOOK_MODE = $mode
        $log = Join-Path $Out "run-$mode-$run.txt"
        $err = Join-Path $Out "run-$mode-$run.err"
        $started = Get-Date
        $process = Start-Process $Exe -ArgumentList @('--do', "@$here\steps.txt", "$here\app.tsx") -WorkingDirectory $here -WindowStyle Hidden -PassThru -RedirectStandardOutput $log -RedirectStandardError $err
        $null = $process.Handle
        $process.WaitForExit()
        $took = ((Get-Date) - $started).TotalSeconds
        $clicks = @(); $wheels = @(); $memory = @()
        $step = ''
        foreach ($line in Get-Content $log) {
            if ($line -like '> *') { $step = $line.Substring(2) }
            elseif ($line -match '^\s+took ([\d.]+) ms') {
                if ($step -like 'click*') { $clicks += [double]$Matches[1] }
                elseif ($step -like 'wheel*') { $wheels += [double]$Matches[1] }
            }
            elseif ($line -match 'private (\d+) B') { $memory += [math]::Round([double]$Matches[1] / 1MB, 1) }
        }
        $median = { param($list) if ($list.Count -eq 0) { 'n/a' } else { $sorted = $list | Sort-Object; $sorted[[int][math]::Floor($sorted.Count / 2)] } }
        "{0,-6} run {1}: {2,6:N2} s in all, click median {3} ms, wheel median {4} ms, private MB {5}" -f $mode, $run, $took, (& $median $clicks), (& $median $wheels), ($memory -join ' / ')
    }
}
