<#
Makes an APK of an rsui app, for arm64 phones, with no Gradle: the rsui-android library cross-compiled with the NDK, the
app's files as assets with a list of them, rsui's own activity (crates/rsui-android/java) compiled with javac and d8,
and a manifest, then aapt2, zipalign and apksigner from the Android SDK. Install it with -Install, on the phone adb sees.

    tools/android/build-apk.ps1 -App demos/mobile-hello [-Install] [-Release]

Who the app is comes from its rsui.config.ts (id, name, version, icon, permissions, android.minSdk/targetSdk/orientation,
files.exclude), as `rsui-tsx config --platform android` prints it; -Package and -Label override the id and the name. That
rsui-tsx is -Tsx, or this tree's target\release or target\debug one, or one built for it.

What it needs: the Android SDK (ANDROID_HOME, or %LOCALAPPDATA%\Android\Sdk) with an NDK, build-tools and a platform, a
JDK for javac, apksigner and keytool (JAVA_HOME, or Android Studio's), and Rust's aarch64-linux-android target. The build
runs at low priority, two jobs at a time.
#>
param(
    [Parameter(Mandatory)] [string] $App,
    [string] $Out = '',
    [string] $Package = '',
    [string] $Label = '',
    [string] $Tsx = '',
    [switch] $Install,
    [switch] $Release
)
$ErrorActionPreference = 'Stop'
(Get-Process -Id $PID).PriorityClass = 'BelowNormal'

$root = Resolve-Path "$PSScriptRoot\..\.."
$appPath = (Resolve-Path $App).Path
$name = Split-Path $appPath -Leaf
$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { "$env:LOCALAPPDATA\Android\Sdk" }
$ndk = Get-ChildItem "$sdk\ndk" -Directory | Sort-Object Name | Select-Object -Last 1
$tools = Get-ChildItem "$sdk\build-tools" -Directory | Sort-Object { [version]($_.Name -replace '-.*', '') } | Select-Object -Last 1
$platform = Get-ChildItem "$sdk\platforms" -Directory | Where-Object { Test-Path "$($_.FullName)\android.jar" } | Sort-Object Name | Select-Object -Last 1
$java = if ($env:JAVA_HOME) { $env:JAVA_HOME } else { 'C:\Program Files\Android\Android Studio\jbr' }
if (-not $ndk -or -not $tools -or -not $platform) { throw "the Android SDK at $sdk needs an NDK, build-tools and a platform" }
$env:JAVA_HOME = $java
$env:PATH = "$java\bin;$env:PATH"

# What the app's config says, for Android, from an rsui-tsx that runs here.
if (-not $Tsx) {
    $Tsx = @("$root\target\release\rsui-tsx.exe", "$root\target\debug\rsui-tsx.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $Tsx) {
    Get-ChildItem env: | Where-Object Name -like 'CARGO_PROFILE_*' | ForEach-Object { Remove-Item "env:$($_.Name)" }
    New-Item -ItemType Directory -Force "$root\target\android" | Out-Null
    $p = Start-Process cargo -ArgumentList @('build', '-j', '2', '-p', 'rsui-tsx') -WorkingDirectory $root -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput "$root\target\android\tsx.out" -RedirectStandardError "$root\target\android\tsx.err"
    $null = $p.Handle
    $p.WaitForExit()
    if ($p.ExitCode -ne 0) { Get-Content "$root\target\android\tsx.err" -Tail 30; throw 'rsui-tsx didn''t build, for reading the config' }
    $Tsx = "$root\target\debug\rsui-tsx.exe"
}
$said = & $Tsx config --platform android $appPath
if ($LASTEXITCODE -ne 0) { throw "the app's rsui.config.ts isn't right (above)" }
$config = ($said -join "`n") | ConvertFrom-Json
if (-not $Package) { $Package = $config.id }
if (-not $Label) { $Label = $config.name }
# What's written into the manifest is XML.
function ConvertTo-XmlText([string] $text) { [System.Security.SecurityElement]::Escape($text) }

# A glob, as the config has them, as a regular expression on a path with `/` between its parts.
function ConvertTo-Pattern([string] $glob) {
    $glob = $glob -replace '^\./', ''
    $out = '^'
    for ($i = 0; $i -lt $glob.Length; $i++) {
        $c = $glob[$i]
        if ($c -eq '*' -and $i + 1 -lt $glob.Length -and $glob[$i + 1] -eq '*') {
            $i++
            if ($i + 1 -lt $glob.Length -and $glob[$i + 1] -eq '/') { $i++; $out += '(?:.*/)?' } else { $out += '.*' }
        } elseif ($c -eq '*') { $out += '[^/]*' }
        elseif ($c -eq '?') { $out += '[^/]' }
        else { $out += [regex]::Escape([string]$c) }
    }
    $out + '$'
}
$excluded = @($config.files.exclude | ForEach-Object { ConvertTo-Pattern $_ })

$work = "$root\target\android\$name"
Remove-Item -Recurse -Force "$work\stage" -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$work\stage\assets", "$work\stage\lib\arm64-v8a" | Out-Null

# The library, built for arm64 Android with the NDK's linker for Android 10 and up.
$clang = "$($ndk.FullName)\toolchains\llvm\prebuilt\windows-x86_64\bin"
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = "$clang\aarch64-linux-android29-clang.cmd"
$env:CC_aarch64_linux_android = "$clang\aarch64-linux-android29-clang.cmd"
$env:AR_aarch64_linux_android = "$clang\llvm-ar.exe"
# Page-aligned to 16 KB, as Android 15's 16 KB-page phones and Google Play need.
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS = '-C link-arg=-Wl,-z,max-page-size=16384'
Get-ChildItem env: | Where-Object Name -like 'CARGO_PROFILE_*' | ForEach-Object { Remove-Item "env:$($_.Name)" }
if ($Release) {
    $buildProfile = 'release'
} else {
    # Quick to build and fast enough to judge by: release without whole-program optimizing.
    $buildProfile = 'fastrel'
    $env:CARGO_PROFILE_FASTREL_INHERITS = 'release'
    $env:CARGO_PROFILE_FASTREL_LTO = 'false'
    $env:CARGO_PROFILE_FASTREL_CODEGEN_UNITS = '16'
    $env:CARGO_PROFILE_FASTREL_INCREMENTAL = 'true'
}
Push-Location $root
try {
    $p = Start-Process cargo -ArgumentList @('build', '-j', '2', '-p', 'rsui-android', '--target', 'aarch64-linux-android', '--profile', $buildProfile) `
        -WindowStyle Hidden -PassThru -RedirectStandardOutput "$work\cargo.out" -RedirectStandardError "$work\cargo.err"
    $null = $p.Handle
    $p.WaitForExit()
    if ($p.ExitCode -ne 0) { Get-Content "$work\cargo.err" -Tail 30; throw 'the library didn''t build' }
} finally { Pop-Location }
Copy-Item "$root\target\aarch64-linux-android\$buildProfile\librsui_app.so" "$work\stage\lib\arm64-v8a\librsui_app.so"

# rsui's activity, as Android's own code: Java's classes, then d8's dex of them.
$classes = "$work\classes"
Remove-Item -Recurse -Force $classes -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $classes | Out-Null
$sources = @(Get-ChildItem "$root\crates\rsui-android\java" -Recurse -Filter *.java | ForEach-Object FullName)
# javac's notes come on stderr, which PowerShell 5.1 would stop at; only how it ends says whether it failed.
$ErrorActionPreference = 'Continue'
& "$java\bin\javac.exe" -nowarn --release 11 -classpath "$($platform.FullName)\android.jar" -d $classes $sources 2>&1 | ForEach-Object { "$_" }
$compiledOk = $LASTEXITCODE -eq 0
$ErrorActionPreference = 'Stop'
if (-not $compiledOk) { throw 'javac failed' }
$compiled = @(Get-ChildItem $classes -Recurse -Filter *.class | ForEach-Object FullName)
& "$($tools.FullName)\d8.bat" --release --min-api 29 --lib "$($platform.FullName)\android.jar" --output $work $compiled
if ($LASTEXITCODE -ne 0) { throw 'd8 failed' }

# The app's files, and the list of them, which is what the library copies out on the phone.
$files = Get-ChildItem $appPath -Recurse -File | Where-Object {
    $_.FullName -notmatch '\\(\.git|screenshots|target)\\' -and $_.Extension -notin '.rsuitrace', '.exe', '.png' -or $_.FullName -match '\\(assets|public)\\'
}
$list = foreach ($file in $files) {
    $relative = $file.FullName.Substring($appPath.Length + 1)
    $slashed = $relative -replace '\\', '/'
    if ($excluded | Where-Object { $slashed -match $_ }) { continue }
    $to = "$work\stage\assets\$relative"
    New-Item -ItemType Directory -Force (Split-Path $to) | Out-Null
    Copy-Item $file.FullName $to
    $relative -replace '\\', '/'
}
[System.IO.File]::WriteAllLines("$work\stage\assets\rsui-assets.txt", [string[]]$list)

# The launcher's icon, from the config's .png, as Android's own resource, which it scales for each screen.
$resources = @()
$iconAttribute = ''
if ($config.icon) {
    if ($config.icon -match '\.png$') {
        Remove-Item -Recurse -Force "$work\res" -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Force "$work\res\mipmap" | Out-Null
        Copy-Item $config.icon "$work\res\mipmap\ic_launcher.png"
        & "$($tools.FullName)\aapt2.exe" compile --dir "$work\res" -o "$work\res.zip"
        if ($LASTEXITCODE -ne 0) { throw 'aapt2 couldn''t take the icon' }
        $resources = @("$work\res.zip")
        $iconAttribute = ' android:icon="@mipmap/ic_launcher"'
    } else {
        Write-Warning "the icon is $($config.icon); Android's launcher takes a .png, so the APK has none"
    }
}
$orientation = @{ any = 'unspecified'; portrait = 'portrait'; landscape = 'landscape' }[$config.android.orientation]
$permissions = ($config.android.permissions | ForEach-Object { "    <uses-permission android:name=`"$_`" />" }) -join "`n"
# A build that isn't -Release can be looked into, as Gradle's debug builds can: `adb shell run-as <id>` reaches its folder,
# where an `env` file sets rsui's switches (see rsui-android).
$debuggable = if ($Release) { '' } else { ' android:debuggable="true"' }
$manifest = @"
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="$Package" android:versionCode="$($config.android.versionCode)" android:versionName="$(ConvertTo-XmlText $config.version)">
    <uses-sdk android:minSdkVersion="$($config.android.minSdk)" android:targetSdkVersion="$($config.android.targetSdk)" />
$permissions
    <application android:label="$(ConvertTo-XmlText $Label)"$iconAttribute android:hasCode="true" android:extractNativeLibs="false"$debuggable>
        <activity android:name="dev.rsui.RsuiActivity" android:exported="true" android:launchMode="singleTask" android:screenOrientation="$orientation"
                  android:configChanges="orientation|screenSize|screenLayout|smallestScreenSize|keyboard|keyboardHidden|uiMode|density|fontScale"
                  android:windowSoftInputMode="adjustResize" android:theme="@android:style/Theme.DeviceDefault.NoActionBar">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
"@
[System.IO.File]::WriteAllText("$work\AndroidManifest.xml", $manifest)

$bin = $tools.FullName
& "$bin\aapt2.exe" link -o "$work\made.apk" --manifest "$work\AndroidManifest.xml" -I "$($platform.FullName)\android.jar" -A "$work\stage\assets" @resources
if ($LASTEXITCODE -ne 0) { throw 'aapt2 failed' }
# The library goes in uncompressed, so Android can map it from the APK as it is; the activity's dex beside it.
# A new zip of what aapt2 made, each entry as it was (resources.arsc has to stay uncompressed), and the two more: appending
# to aapt2's own zip leaves headers zipalign won't take once there are resources.
python -c @"
import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as made, zipfile.ZipFile(sys.argv[4], 'w') as apk:
    for entry in made.infolist():
        apk.writestr(entry.filename, made.read(entry), compress_type=entry.compress_type)
    apk.write(sys.argv[2], 'lib/arm64-v8a/librsui_app.so', compress_type=zipfile.ZIP_STORED)
    apk.write(sys.argv[3], 'classes.dex', compress_type=zipfile.ZIP_DEFLATED)
"@ "$work\made.apk" "$work\stage\lib\arm64-v8a\librsui_app.so" "$work\classes.dex" "$work\base.apk"
if ($LASTEXITCODE -ne 0) { throw 'adding the library failed' }
& "$bin\zipalign.exe" -f -P 16 4 "$work\base.apk" "$work\aligned.apk"
if ($LASTEXITCODE -ne 0) { throw 'zipalign failed' }

# Signed with a key made for this machine's builds, kept with them.
$keystore = "$root\target\android\debug.keystore"
if (-not (Test-Path $keystore)) {
    & "$java\bin\keytool.exe" -genkeypair -keystore $keystore -storepass android -keypass android -alias rsuidebug `
        -keyalg RSA -keysize 2048 -validity 10000 -dname 'CN=rsui debug' | Out-Null
}
if (-not $Out) { $Out = "$work\$name.apk" }
& "$bin\apksigner.bat" sign --ks $keystore --ks-pass pass:android --key-pass pass:android --out $Out "$work\aligned.apk"
if ($LASTEXITCODE -ne 0) { throw 'apksigner failed' }
"APK: $Out, $Package $($config.version) ($([math]::Round((Get-Item $Out).Length / 1MB, 2)) MB, library $([math]::Round((Get-Item "$work\stage\lib\arm64-v8a\librsui_app.so").Length / 1MB, 2)) MB)"

if ($Install) {
    & "$sdk\platform-tools\adb.exe" install -r $Out
    & "$sdk\platform-tools\adb.exe" shell am start -n "$Package/dev.rsui.RsuiActivity"
}
