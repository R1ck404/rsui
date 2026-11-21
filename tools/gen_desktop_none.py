# Writes crates/rsui/src/desktop_none.rs: the public types of desktop.rs, desktop/more.rs and desktop/sound.rs copied as they
# are, and each public function as one that says it isn't on this system yet, for systems without their own desktop code.
import io, re, sys

ROOT = sys.argv[1]
FILES = ['desktop.rs', 'desktop/more.rs', 'desktop/sound.rs']
# Functions whose bodies don't touch Windows, copied as they are, with the private ones they need.
KEEP = {'natural_order', 'natural_order_ascii'}

# What these do on Unix already, rather than nothing.
OVERRIDES = {
    # The window says which app it is itself, by the program's name, which is all there is to it here.
    'set_app_id': '    Ok(())\n',
    'message_box': '    // Nothing could be asked, so nothing was agreed to.\n    "cancel"\n',
    'battery': '    Battery { on_battery: false, level: None, charging: false }\n',
    'accessibility': '    Accessibility { reduced_motion: false, high_contrast: false, text_scale: 1.0 }\n',
    'network_status': '    // Not knowing, the network is taken to be there, as most programs would rather.\n    NetworkStatus { online: true, local_only: false, metered: false }\n',
    'temp_folder': '    std::env::temp_dir()\n',
    'locale': '''    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty() && value != "C" && value != "POSIX")
        .map(|value| value.split(['.', '@']).next().unwrap_or_default().replace('_', "-"))
        .unwrap_or_else(|| "en-US".to_owned())
''',
    'time_zone': '''    if let Ok(zone) = std::env::var("TZ") {
        return Some(zone.trim_start_matches(':').to_owned()).filter(|zone| !zone.is_empty());
    }
    // `/etc/localtime` is a link into the zone files, whose path ends in the zone's name.
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let text = target.to_string_lossy();
    text.split_once("zoneinfo/").map(|(_, name)| name.to_owned())
''',
    'open_path': '    open_with_system(path.as_os_str())\n',
    'reveal': '    open_with_system(path.parent().unwrap_or(path).as_os_str())\n',
    'list_folder': '''    use std::os::unix::fs::PermissionsExt;
    let millis = |time: std::io::Result<std::time::SystemTime>| {
        time.ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0.0, |since| since.as_secs_f64() * 1000.0)
    };
    let entries = std::fs::read_dir(path).map_err(|error| format!("can't read {}: {error}", path.display()))?;
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(link) = entry.file_type() else { continue };
        // What a link points to is what's shown, as a file manager does, and a link that points nowhere is shown as itself.
        let Ok(metadata) = std::fs::metadata(entry.path()).or_else(|_| entry.metadata()) else { continue };
        found.push(FolderEntry {
            hidden: name.starts_with('.'),
            name,
            folder: metadata.is_dir(),
            size: metadata.len(),
            modified: millis(metadata.modified()),
            created: millis(metadata.created()),
            system: false,
            readonly: metadata.permissions().mode() & 0o222 == 0,
            link: link.is_symlink(),
        });
    }
    found.sort_by(|a, b| natural_order(&a.name, &b.name).then_with(|| a.name.cmp(&b.name)));
    Ok(found)
''',
    'drives': '''    // There are no drives by letter: the whole of the file system starts at `/`, and the person's own things are in home.
    let mut found = vec![Drive { path: "/".to_owned(), label: String::new(), kind: "fixed", total: 0, free: 0 }];
    if let Some(home) = std::env::var_os("HOME") {
        found.push(Drive { path: home.to_string_lossy().into_owned(), label: "Home".to_owned(), kind: "fixed", total: 0, free: 0 });
    }
    found
''',
}

EXTRA = '''
/// Has the system open something with whatever it opens such things with: `xdg-open` on Linux, `open` on macOS.
fn open_with_system(what: &std::ffi::OsStr) -> Result<(), String> {
    let opener = if cfg!(target_vendor = "apple") { "open" } else { "xdg-open" };
    std::process::Command::new(opener)
        .arg(what)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("{opener} couldn't be started: {error}"))
}
'''

def read(path):
    return io.open(ROOT + '/' + path, encoding='utf-8', newline='').read().replace('\r\n', '\n')

def block_end(text, start):
    """Index just past the item starting at `start`: through its matching `}` or its `;`."""
    depth = 0
    i = start
    in_str = False
    while i < len(text):
        c = text[i]
        if c == '"' and text[i-1] != '\\' and text[i-1] != "'":
            in_str = not in_str
        elif not in_str:
            if c == '{':
                depth += 1
            elif c == '}':
                depth -= 1
                if depth == 0:
                    return i + 1
            elif c == ';' and depth == 0:
                return i + 1
        i += 1
    return len(text)

def leading(text, start):
    """Where the doc comments and attributes before `start` begin."""
    lines_before = text[:start].split('\n')
    # the last element is the current line's indentation (empty for top level)
    keep = []
    for line in reversed(lines_before[:-1]):
        s = line.strip()
        if s.startswith('///') or s.startswith('#[') or (s.startswith('//') and not s.startswith('//!')):
            keep.append(line)
        else:
            break
    return start - sum(len(l) + 1 for l in keep)

def default_for(ret):
    ret = ret.strip()
    if ret == '':
        return ''
    if ret.startswith('Result<'):
        return 'Err(NOT_HERE.to_owned())'
    if ret.startswith('Option<'):
        return 'None'
    if ret == 'bool':
        return 'false'
    if ret.startswith('Vec<'):
        return 'Vec::new()'
    if ret == 'String':
        return 'String::new()'
    if ret in ('u32', 'i32', 'u64', 'usize'):
        return '0'
    if ret == '(i32, i32)':
        return '(0, 0)'
    if ret == 'PathBuf':
        return 'std::env::temp_dir()'
    if ret == 'SoundId':
        return 'SoundId(0)'
    return ret + '::default()'

out = []
out.append('''//! The desktop's services on systems that don't have their own written yet: the same types and functions as on Windows
//! (`desktop.rs`), each of which says it can't be done here, or gives what's least surprising, so that a program written
//! for them still builds and runs, without them. This file is written by tools/gen_desktop_none.py from the Windows ones, so that the two
//! stay alike; each system's real ones replace these as they're written.
#![allow(unused_variables, dead_code, clippy::all)]

#[path = "desktop/keys.rs"]
mod keys;

use std::path::{Path, PathBuf};
use std::rc::Rc;

pub use keys::{Keys, parse_keys};

/// What each of these says when it can't be done here.
const NOT_HERE: &str = "this isn't on this system yet";
''')

for name in FILES:
    text = read(name)
    pos = 0
    pattern = re.compile(r'^(pub (?:struct|enum|type|fn) |pub\(crate\) fn |fn |impl(?:<[^>]*>)? [A-Za-z]|impl [A-Za-z])', re.M)
    for m in pattern.finditer(text):
        start = m.start()
        if start < pos:
            continue
        end = block_end(text, start)
        head = leading(text, start)
        item = text[head:end]
        kind = m.group(1)
        pos = end
        if kind.startswith('impl'):
            first = text[start:text.index('{', start)]
            body = text[start:end]
            windowsy = re.search(r'unsafe|windows::|HWND|HICON|PCWSTR|w!\(|s!\(|HSTRING|\bWin32\b', body)
            if not windowsy and re.search(r'impl(?:<[^>]*>)? (?:Default|std::fmt::Debug|Clone|PartialEq|From<)', first) or (
                not windowsy and re.match(r'impl [A-Z][A-Za-z]* \{', first)):
                out.append('\n' + item + '\n')
            continue
        if kind.startswith('pub struct') or kind.startswith('pub enum') or kind.startswith('pub type'):
            out.append('\n' + item + '\n')
            continue
        # A function: its signature up to the body.
        brace = text.index('{', start)
        signature = text[head:brace].rstrip()
        fn_name = re.search(r'fn ([a-z_0-9]+)', text[start:brace]).group(1)
        if fn_name in KEEP:
            out.append('\n' + item + '\n')
            continue
        if kind.startswith('pub(crate)') or kind.startswith('fn '):
            continue
        # The signature without its comments, which describe what Windows does, and with what's left of its attributes.
        signature = '\n'.join(line for line in signature.split('\n') if not line.strip().startswith('//'))
        if fn_name in OVERRIDES:
            out.append('\n' + signature + ' {\n' + OVERRIDES[fn_name] + '}\n')
            continue
        ret = ''
        arrow = text[start:brace].rfind('->')
        if arrow >= 0:
            ret = text[start:brace][arrow + 2:]
            ret = re.sub(r'\s+where.*', '', ret, flags=re.S)
        body = default_for(ret)
        out.append('\n/// Not on this system yet.\n' + signature + ' {\n' + ('    ' + body + '\n' if body else '') + '}\n')

out.append(EXTRA)
io.open(ROOT + '/desktop_none.rs', 'w', encoding='utf-8', newline='\r\n').write(''.join(out))
print('written', len(out), 'items')
