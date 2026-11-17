//! Node's `path.posix`, step for step as Node 24 has it in `lib/path.js`, on the text's UTF-16 units. Only `/` separates
//! parts here, and only a path that starts with one is absolute. See `path_win32.rs` for what it shares with Windows'.

use super::path_win32::{basename_from, last_part, normalize_string};

const SLASH: u16 = b'/' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const DOT: u16 = b'.' as u16;

/// The working folder as `path.posix` sees it: on Windows, with its slashes turned and its drive left out, as
/// `/Users/...`.
fn cwd(ask: &mut dyn FnMut() -> Vec<u16>) -> Vec<u16> {
    let mut cwd = ask();
    if cfg!(windows) {
        for unit in &mut cwd {
            if *unit == BACKSLASH {
                *unit = SLASH;
            }
        }
        // `indexOf` giving -1 makes `slice` start from one before the end.
        match cwd.iter().position(|unit| *unit == SLASH) {
            Some(at) => {
                cwd.drain(..at);
            }
            None => {
                cwd.drain(..cwd.len().saturating_sub(1));
            }
        }
    }
    cwd
}

pub(super) fn resolve(args: &[&[u16]], ask: &mut dyn FnMut() -> Vec<u16>) -> Vec<u16> {
    if args.is_empty() || (args.len() == 1 && (args[0].is_empty() || args[0] == [DOT])) {
        let cwd = cwd(ask);
        if cwd.first() == Some(&SLASH) {
            return cwd;
        }
    }
    let mut resolved: Vec<u16> = Vec::new();
    let mut absolute = false;
    for path in args.iter().rev() {
        if absolute {
            break;
        }
        if path.is_empty() {
            continue;
        }
        let mut joined = path.to_vec();
        joined.push(SLASH);
        joined.extend_from_slice(&resolved);
        resolved = joined;
        absolute = path[0] == SLASH;
    }
    if !absolute {
        let mut joined = cwd(ask);
        absolute = joined.first() == Some(&SLASH);
        joined.push(SLASH);
        joined.extend_from_slice(&resolved);
        resolved = joined;
    }
    let normalized = normalize_string(&resolved, !absolute, false);
    if absolute {
        let mut out = vec![SLASH];
        out.extend_from_slice(&normalized);
        out
    } else if normalized.is_empty() {
        vec![DOT]
    } else {
        normalized
    }
}

pub(super) fn normalize(path: &[u16]) -> Vec<u16> {
    if path.is_empty() {
        return vec![DOT];
    }
    let absolute = path[0] == SLASH;
    let trailing = path[path.len() - 1] == SLASH;
    let mut normalized = normalize_string(path, !absolute, false);
    if normalized.is_empty() {
        return if absolute {
            vec![SLASH]
        } else if trailing {
            vec![DOT, SLASH]
        } else {
            vec![DOT]
        };
    }
    if trailing {
        normalized.push(SLASH);
    }
    if absolute {
        normalized.insert(0, SLASH);
    }
    normalized
}

pub(super) fn is_absolute(path: &[u16]) -> bool {
    path.first() == Some(&SLASH)
}

pub(super) fn join(args: &[&[u16]]) -> Vec<u16> {
    let mut joined: Vec<u16> = Vec::new();
    let mut any = false;
    for path in args.iter().filter(|path| !path.is_empty()) {
        if any {
            joined.push(SLASH);
        }
        joined.extend_from_slice(path);
        any = true;
    }
    if !any {
        return vec![DOT];
    }
    normalize(&joined)
}

pub(super) fn relative(from: &[u16], to: &[u16], ask: &mut dyn FnMut() -> Vec<u16>) -> Vec<u16> {
    if from == to {
        return Vec::new();
    }
    let from = resolve(&[from], ask);
    let to = resolve(&[to], ask);
    if from == to {
        return Vec::new();
    }
    let from_start = 1;
    let from_end = from.len();
    let from_len = from_end - from_start;
    let to_start = 1;
    let to_len = to.len() - to_start;
    let length = from_len.min(to_len);
    let mut last_common_sep: isize = -1;
    let mut i = 0;
    while i < length {
        let from_code = from[from_start + i];
        if from_code != to[to_start + i] {
            break;
        } else if from_code == SLASH {
            last_common_sep = i as isize;
        }
        i += 1;
    }
    if i == length {
        if to_len > length {
            if to[to_start + i] == SLASH {
                // `from` is what `to` is in.
                return to[to_start + i + 1..].to_vec();
            }
            if i == 0 {
                // `from` is the root.
                return to[to_start + i..].to_vec();
            }
        } else if from_len > length {
            if from[from_start + i] == SLASH {
                last_common_sep = i as isize;
            } else if i == 0 {
                last_common_sep = 0;
            }
        }
    }
    let mut out: Vec<u16> = Vec::new();
    let mut index = from_start as isize + last_common_sep + 1;
    while index <= from_end as isize {
        if index == from_end as isize || from[index as usize] == SLASH {
            if out.is_empty() {
                out.extend_from_slice(&[DOT, DOT]);
            } else {
                out.extend_from_slice(&[SLASH, DOT, DOT]);
            }
        }
        index += 1;
    }
    let rest = (to_start as isize + last_common_sep) as usize;
    out.extend_from_slice(&to[rest.min(to.len())..]);
    out
}

pub(super) fn dirname(path: &[u16]) -> Vec<u16> {
    if path.is_empty() {
        return vec![DOT];
    }
    let has_root = path[0] == SLASH;
    let mut end = None;
    let mut matched_slash = true;
    for i in (1..path.len()).rev() {
        if path[i] == SLASH {
            if !matched_slash {
                end = Some(i);
                break;
            }
        } else {
            matched_slash = false;
        }
    }
    match end {
        None if has_root => vec![SLASH],
        None => vec![DOT],
        Some(1) if has_root => vec![SLASH, SLASH],
        Some(end) => path[..end].to_vec(),
    }
}

pub(super) fn basename<'a>(path: &'a [u16], suffix: Option<&[u16]>) -> &'a [u16] {
    basename_from(path, suffix, 0, false)
}

pub(super) fn extname(path: &[u16]) -> &[u16] {
    match last_part(path, 0, 0, false) {
        (_, Some(dot), Some(end)) => &path[dot..end],
        _ => &[],
    }
}

/// `parse`'s root, dir, base, ext and name.
pub(super) fn parse(path: &[u16]) -> [Vec<u16>; 5] {
    if path.is_empty() {
        return Default::default();
    }
    let absolute = path[0] == SLASH;
    let root: &[u16] = if absolute { &[SLASH] } else { &[] };
    let (start_part, dot, end) = last_part(path, usize::from(absolute), 0, false);
    let (mut base, mut ext, mut name): (&[u16], &[u16], &[u16]) = (&[], &[], &[]);
    if let Some(end) = end {
        let start = if start_part == 0 && absolute { 1 } else { start_part };
        base = &path[start..end];
        match dot {
            Some(dot) => {
                name = &path[start..dot];
                ext = &path[dot..end];
            }
            None => name = base,
        }
    }
    let dir: &[u16] = if start_part > 0 {
        &path[..start_part - 1]
    } else if absolute {
        &[SLASH]
    } else {
        &[]
    };
    [root.to_vec(), dir.to_vec(), base.to_vec(), ext.to_vec(), name.to_vec()]
}
