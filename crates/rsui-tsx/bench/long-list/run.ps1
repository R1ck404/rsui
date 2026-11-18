# Runs the long-list bench headless, sandboxed, once for each row count, and says how long each step took and where its time
# went. -Exe is the rsui-tsx to run, -Counts the row counts, -Mode "cv" for rows with `content-visibility: auto`, -Out where
# each run's output goes.
param(
    [string]$Exe = "$env:CARGO_TARGET_DIR\release\rsui-tsx.exe",
    [int[]]$Counts = @(1000, 10000, 25000),
    [string]$Mode = "",
    [string]$Out = "$env:TEMP\long-list-bench",
    [int]$Limit = 300
)
(Get-Process -Id $PID).PriorityClass = 'BelowNormal'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
New-Item -ItemType Directory -Force $Out | Out-Null
foreach ($count in $Counts) {
    $sandbox = Join-Path $Out "sandbox-$count$Mode"
    New-Item -ItemType Directory -Force "$sandbox\roaming", "$sandbox\local" | Out-Null
    $env:RSUI_SANDBOX = $sandbox
    $env:APPDATA = "$sandbox\roaming"
    $env:LOCALAPPDATA = "$sandbox\local"
    $env:RSUI_OFFLINE = '1'
    $env:RSUI_STEP_TIMES = '1'
    $env:RSUI_SPLIT = '1'
    $env:RSUI_DO_SETTLE_MS = '0'
    $env:LONG_LIST_N = "$count"
    $env:LONG_LIST_MODE = $Mode
    $log = Join-Path $Out "run-$count$Mode.txt"
    $err = Join-Path $Out "run-$count$Mode.err"
    $started = Get-Date
    $process = Start-Process $Exe -ArgumentList @('--do', "@$here\steps.txt", "$here\app.tsx") -WorkingDirectory $here -WindowStyle Hidden -PassThru -RedirectStandardOutput $log -RedirectStandardError $err
    if (-not $process.WaitForExit($Limit * 1000)) {
        Stop-Process -Id $process.Id -Force
        "$count${Mode}: stopped after $Limit s"
        continue
    }
    $took = ((Get-Date) - $started).TotalSeconds
    "== $count rows $Mode (exit $($process.ExitCode), $([math]::Round($took, 1)) s in all)"
    $step = ''
    $steps = 0.0
    foreach ($line in Get-Content $log) {
        if ($line -like '> *') { $step = $line.Substring(2) }
        elseif ($line -match '^\s+took ([\d.]+) ms') { $steps += [double]$Matches[1] }
        if ($line -match '^\s+(took|pressed) ([\d.]+) ms') {
            if ($Matches[1] -eq 'took' -and $step -like 'press*') { continue }
            "{0,10} ms  {1}" -f $Matches[2], $step
        }
        elseif ($line -match 'process: working set (\d+) B \(peak (\d+) B\), private working set (\d+) B, private (\d+) B') {
            "{0,10} MB  working set, {1} MB private ({2})" -f [math]::Round($Matches[1] / 1MB, 1), [math]::Round($Matches[4] / 1MB, 1), $step
        }
    }
    # What isn't any step is starting, with the app's first render and layout, and ending.
    "{0,10} ms  starting and ending" -f [math]::Round($took * 1000 - $steps)
    Get-Content $err | Select-Object -First 5
}
