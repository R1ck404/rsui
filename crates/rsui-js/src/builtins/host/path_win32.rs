//! Node's `path.win32`, step for step as Node 24 has it in `lib/path.js`, on the text's UTF-16 units. Windows' paths have
//! more kinds of root than a drive's (`C:` on its own, which is the folder that drive was last in, network shares,
//! `\\.\` and `\\?\` devices, and names like `CON:` that are devices wherever they are), and Node's handling of each has
//! grown case by case, with some of it there to keep a path that wasn't absolute from becoming one. Following its steps
//! is the only way to be sure of giving what it gives.

const SLASH: u16 = b'/' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const DOT: u16 = b'.' as u16;
const COLON: u16 = b':' as u16;
const QUESTION: u16 = b'?' as u16;

/// What [`resolve`] and [`relative`] ask of the program: the working folder for `None`, and an environment variable
/// for a name, which is the folder a drive was last in for `=C:`. Nothing, or an empty one, is the same as not set.
pub(super) type Ask<'a> = dyn FnMut(Option<&[u16]>) -> Vec<u16> + 'a;

fn is_separator(unit: u16) -> bool {
    unit == SLASH || unit == BACKSLASH
}

fn at(path: &[u16], index: usize) -> Option<u16> {
    path.get(index).copied()
}

fn separator_at(path: &[u16], index: usize) -> bool {
    at(path, index).is_some_and(is_separator)
}

fn is_device_root(unit: u16) -> bool {
    (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) || (u16::from(b'a')..=u16::from(b'z')).contains(&unit)
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn index_of(path: &[u16], unit: u16, from: usize) -> Option<usize> {
    path.iter().skip(from).position(|found| *found == unit).map(|found| found + from)
}

/// JavaScript's `slice`, where an end below zero counts from the end, as Node's code relies on for `indexOf` giving -1.
fn slice(path: &[u16], start: isize, end: isize) -> &[u16] {
    let length = path.len() as isize;
    let clamp = |at: isize| if at < 0 { (length + at).max(0) } else { at.min(length) } as usize;
    let (start, end) = (clamp(start), clamp(end));
    if start >= end { &[] } else { &path[start..end] }
}

/// JavaScript's `toLowerCase` or `toUpperCase`, which can change how long the text is.
fn change_case(path: &[u16], upper: bool) -> Vec<u16> {
    match String::from_utf16(path) {
        Ok(text) => units(&if upper { text.to_uppercase() } else { text.to_lowercase() }),
        // Lone surrogates stay as they are, as they do in JavaScript.
        Err(_) => char::decode_utf16(path.iter().copied())
            .flat_map(|found| match found {
                Ok(character) => {
                    let changed: String = if upper {
                        character.to_uppercase().collect()
                    } else {
                        character.to_lowercase().collect()
                    };
                    units(&changed)
                }
                Err(lone) => vec![lone.unpaired_surrogate()],
            })
            .collect(),
    }
}

fn lower(path: &[u16]) -> Vec<u16> {
    change_case(path, false)
}

const RESERVED_NAMES: [&str; 28] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "COM\u{b9}", "COM\u{b2}", "COM\u{b3}", "LPT\u{b9}",
    "LPT\u{b2}", "LPT\u{b3}",
];

/// Whether what's before `colon` is a device's name, as `CON` is. `colon` is what `indexOf` gave, and can be -1.
fn is_reserved_name(path: &[u16], colon: isize) -> bool {
    let device = change_case(slice(path, 0, colon), true);
    RESERVED_NAMES.iter().any(|name| units(name) == device)
}

fn colon_index(path: &[u16]) -> isize {
    index_of(path, COLON, 0).map_or(-1, |found| found as isize)
}

/// Whether this separates parts of a path, of Windows' kind or everyone else's.
fn separates(unit: u16, windows: bool) -> bool {
    unit == SLASH || (windows && unit == BACKSLASH)
}

/// Takes `.` and `..` out of a path that has no root, as Node's `normalizeString` does, for both kinds of path.
pub(super) fn normalize_string(path: &[u16], allow_above_root: bool, windows: bool) -> Vec<u16> {
    let separator = if windows { BACKSLASH } else { SLASH };
    let is_separator = |unit: u16| separates(unit, windows);
    let mut res: Vec<u16> = Vec::new();
    let mut last_segment_length = 0usize;
    let mut last_slash: isize = -1;
    let mut dots: i32 = 0;
    let mut code = 0u16;
    let mut i = 0usize;
    while i <= path.len() {
        if i < path.len() {
            code = path[i];
        } else if is_separator(code) {
            break;
        } else {
            code = SLASH;
        }
        if is_separator(code) {
            if last_slash == i as isize - 1 || dots == 1 {
                // Nothing to do.
            } else if dots == 2 {
                let ends_in_up =
                    res.len() >= 2 && last_segment_length == 2 && res[res.len() - 1] == DOT && res[res.len() - 2] == DOT;
                if !ends_in_up {
                    if res.len() > 2 {
                        let last_slash_index = res.len() as isize - last_segment_length as isize - 1;
                        if last_slash_index == -1 {
                            res.clear();
                            last_segment_length = 0;
                        } else {
                            res.truncate(last_slash_index as usize);
                            last_segment_length = match res.iter().rposition(|unit| *unit == separator) {
                                Some(found) => res.len() - 1 - found,
                                None => res.len(),
                            };
                        }
                        last_slash = i as isize;
                        dots = 0;
                        i += 1;
                        continue;
                    } else if !res.is_empty() {
                        res.clear();
                        last_segment_length = 0;
                        last_slash = i as isize;
                        dots = 0;
                        i += 1;
                        continue;
                    }
                }
                if allow_above_root {
                    if !res.is_empty() {
                        res.push(separator);
                    }
                    res.extend_from_slice(&[DOT, DOT]);
                    last_segment_length = 2;
                }
            } else {
                let part = &path[(last_slash + 1) as usize..i];
                if !res.is_empty() {
                    res.push(separator);
                }
                res.extend_from_slice(part);
                last_segment_length = (i as isize - last_slash - 1) as usize;
            }
            last_slash = i as isize;
            dots = 0;
        } else if code == DOT && dots != -1 {
            dots += 1;
        } else {
            dots = -1;
        }
        i += 1;
    }
    res
}

/// Where a network share's name ends, from the start of a path that starts with two separators: the parts it's made
/// of, `\\server\share`, and how far along the path they reach. `None` if there's no share there.
fn unc_parts(path: &[u16]) -> Option<(&[u16], usize, usize)> {
    let len = path.len();
    let mut j = 2;
    let mut last = j;
    while j < len && !is_separator(path[j]) {
        j += 1;
    }
    if j >= len || j == last {
        return None;
    }
    let first = &path[last..j];
    last = j;
    while j < len && is_separator(path[j]) {
        j += 1;
    }
    if j >= len || j == last {
        return None;
    }
    last = j;
    while j < len && !is_separator(path[j]) {
        j += 1;
    }
    Some((first, last, j))
}

pub(super) fn resolve(args: &[&[u16]], ask: &mut Ask<'_>) -> Vec<u16> {
    let mut resolved_device: Vec<u16> = Vec::new();
    let mut resolved_tail: Vec<u16> = Vec::new();
    let mut resolved_absolute = false;
    let mut i = args.len() as isize - 1;
    while i >= -1 {
        let path: Vec<u16> = if i >= 0 {
            let given = args[i as usize];
            if given.is_empty() {
                i -= 1;
                continue;
            }
            given.to_vec()
        } else if resolved_device.is_empty() {
            let mut cwd = ask(None);
            let only_here = args.is_empty() || (args.len() == 1 && (args[0].is_empty() || args[0] == [DOT]));
            if only_here && separator_at(&cwd, 0) {
                if !cfg!(windows) {
                    for unit in &mut cwd {
                        if *unit == SLASH {
                            *unit = BACKSLASH;
                        }
                    }
                }
                return cwd;
            }
            cwd
        } else {
            // A drive has a folder of its own that it was last in, which Windows keeps in a variable called `=C:`.
            let mut name = vec![u16::from(b'=')];
            name.extend_from_slice(&resolved_device);
            let mut path = ask(Some(&name));
            if path.is_empty() {
                path = ask(None);
            }
            if lower(slice(&path, 0, 2)) != lower(&resolved_device) && at(&path, 2) == Some(BACKSLASH) {
                path = resolved_device.clone();
                path.push(BACKSLASH);
            }
            path
        };
        let len = path.len();
        let mut root_end = 0;
        let mut device: Vec<u16> = Vec::new();
        let mut is_absolute = false;
        let code = at(&path, 0).unwrap_or(0);
        if len == 1 {
            if is_separator(code) {
                root_end = 1;
                is_absolute = true;
            }
        } else if is_separator(code) {
            is_absolute = true;
            if separator_at(&path, 1) {
                if let Some((first, last, j)) = unc_parts(&path) {
                    if j == len || j != last {
                        if first != [DOT] && first != [QUESTION] {
                            device = vec![BACKSLASH, BACKSLASH];
                            device.extend_from_slice(first);
                            device.push(BACKSLASH);
                            device.extend_from_slice(&path[last..j]);
                            root_end = j;
                        } else {
                            device = vec![BACKSLASH, BACKSLASH];
                            device.extend_from_slice(first);
                            root_end = 4;
                        }
                    }
                }
            } else {
                root_end = 1;
            }
        } else if is_device_root(code) && at(&path, 1) == Some(COLON) {
            device = path[..2].to_vec();
            root_end = 2;
            if len > 2 && separator_at(&path, 2) {
                is_absolute = true;
                root_end = 3;
            }
        }
        if !device.is_empty() {
            if !resolved_device.is_empty() {
                if lower(&device) != lower(&resolved_device) {
                    // Another drive's, so it has nothing to do with this one.
                    i -= 1;
                    continue;
                }
            } else {
                resolved_device = device;
            }
        }
        if resolved_absolute {
            if !resolved_device.is_empty() {
                break;
            }
        } else {
            let mut tail = slice(&path, root_end as isize, len as isize).to_vec();
            tail.push(BACKSLASH);
            tail.extend_from_slice(&resolved_tail);
            resolved_tail = tail;
            resolved_absolute = is_absolute;
            if is_absolute && !resolved_device.is_empty() {
                break;
            }
        }
        i -= 1;
    }
    let tail = normalize_string(&resolved_tail, !resolved_absolute, true);
    let mut out = resolved_device;
    if resolved_absolute {
        out.push(BACKSLASH);
        out.extend_from_slice(&tail);
        out
    } else {
        out.extend_from_slice(&tail);
        if out.is_empty() { vec![DOT] } else { out }
    }
}

pub(super) fn normalize(path: &[u16]) -> Vec<u16> {
    let len = path.len();
    if len == 0 {
        return vec![DOT];
    }
    let mut root_end = 0;
    let mut device: Option<Vec<u16>> = None;
    let mut is_absolute = false;
    let code = path[0];
    if len == 1 {
        return if code == SLASH { vec![BACKSLASH] } else { path.to_vec() };
    }
    if is_separator(code) {
        is_absolute = true;
        if separator_at(path, 1) {
            if let Some((first, last, j)) = unc_parts(path) {
                if j == len || j != last {
                    if first == [DOT] || first == [QUESTION] {
                        let mut found = vec![BACKSLASH, BACKSLASH];
                        found.extend_from_slice(first);
                        root_end = 4;
                        // `\\?\COM1:` and the like are devices too.
                        let colon = colon_index(path);
                        let possible = slice(path, 4, colon + 1);
                        if is_reserved_name(possible, possible.len() as isize - 1) {
                            found = units("\\\\?\\");
                            found.extend_from_slice(possible);
                            root_end = 4 + possible.len();
                        }
                        device = Some(found);
                    } else if j == len {
                        // Only a share, which is written out as its root.
                        let mut out = vec![BACKSLASH, BACKSLASH];
                        out.extend_from_slice(first);
                        out.push(BACKSLASH);
                        out.extend_from_slice(&path[last..]);
                        out.push(BACKSLASH);
                        return out;
                    } else {
                        let mut found = vec![BACKSLASH, BACKSLASH];
                        found.extend_from_slice(first);
                        found.push(BACKSLASH);
                        found.extend_from_slice(&path[last..j]);
                        device = Some(found);
                        root_end = j;
                    }
                }
            }
        } else {
            root_end = 1;
        }
    } else {
        let colon = colon_index(path);
        if colon > 0 {
            if is_device_root(code) && colon == 1 {
                device = Some(path[..2].to_vec());
                root_end = 2;
                if len > 2 && separator_at(path, 2) {
                    is_absolute = true;
                    root_end = 3;
                }
            } else if is_reserved_name(path, colon) {
                device = Some(path[..colon as usize + 1].to_vec());
                root_end = colon as usize + 1;
            }
        }
    }
    let mut tail = if root_end < len { normalize_string(&path[root_end..], !is_absolute, true) } else { Vec::new() };
    if tail.is_empty() && !is_absolute {
        tail.push(DOT);
    }
    if !tail.is_empty() && is_separator(path[len - 1]) {
        tail.push(BACKSLASH);
    }
    let dot_first = |tail: &[u16]| {
        let mut out = vec![DOT, BACKSLASH];
        out.extend_from_slice(tail);
        out
    };
    if !is_absolute && device.is_none() && path.contains(&COLON) {
        // What wasn't absolute can't become a path Windows would take as one (CVE-2024-36139).
        if tail.len() >= 2 && is_device_root(tail[0]) && tail[1] == COLON {
            return dot_first(&tail);
        }
        let mut index = index_of(path, COLON, 0);
        while let Some(found) = index {
            if found == len - 1 || separator_at(path, found + 1) {
                return dot_first(&tail);
            }
            index = index_of(path, COLON, found + 1);
        }
    }
    if is_reserved_name(path, colon_index(path)) {
        let mut out = vec![DOT, BACKSLASH];
        out.extend_from_slice(device.as_deref().unwrap_or(&[]));
        out.extend_from_slice(&tail);
        return out;
    }
    let mut out = match device {
        Some(device) => device,
        None => Vec::new(),
    };
    if is_absolute {
        out.push(BACKSLASH);
    }
    out.extend_from_slice(&tail);
    out
}

pub(super) fn is_absolute(path: &[u16]) -> bool {
    let len = path.len();
    if len == 0 {
        return false;
    }
    is_separator(path[0]) || (len > 2 && is_device_root(path[0]) && path[1] == COLON && is_separator(path[2]))
}

pub(super) fn join(args: &[&[u16]]) -> Vec<u16> {
    let paths: Vec<&[u16]> = args.iter().copied().filter(|path| !path.is_empty()).collect();
    if paths.is_empty() {
        return vec![DOT];
    }
    let first = paths[0];
    let mut joined: Vec<u16> = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        if index > 0 {
            joined.push(BACKSLASH);
        }
        joined.extend_from_slice(path);
    }
    // Two separators at the start would make it a network share, unless the first part clearly is one, as
    // `join('//server', 'share')` is.
    let mut needs_replace = true;
    let mut slash_count = 0;
    if separator_at(first, 0) {
        slash_count += 1;
        if first.len() > 1 && separator_at(first, 1) {
            slash_count += 1;
            if first.len() > 2 {
                if separator_at(first, 2) {
                    slash_count += 1;
                } else {
                    needs_replace = false;
                }
            }
        }
    }
    if needs_replace {
        while slash_count < joined.len() && is_separator(joined[slash_count]) {
            slash_count += 1;
        }
        if slash_count >= 2 {
            let mut replaced = vec![BACKSLASH];
            replaced.extend_from_slice(&joined[slash_count..]);
            joined = replaced;
        }
    }
    // A part that names a device, as `CON:` does, is left as it is but for its slashes.
    let has_device = joined
        .split(|unit| *unit == BACKSLASH)
        .filter(|part| !part.is_empty())
        .any(|part| {
            let colon = colon_index(part);
            colon != -1 && is_reserved_name(part, colon)
        });
    if has_device {
        return joined.iter().map(|unit| if *unit == SLASH { BACKSLASH } else { *unit }).collect();
    }
    normalize(&joined)
}

pub(super) fn relative(from: &[u16], to: &[u16], ask: &mut Ask<'_>) -> Vec<u16> {
    if from == to {
        return Vec::new();
    }
    let from_orig = resolve(&[from], ask);
    let to_orig = resolve(&[to], ask);
    if from_orig == to_orig {
        return Vec::new();
    }
    let from = lower(&from_orig);
    let to = lower(&to_orig);
    if from == to {
        return Vec::new();
    }
    if from_orig.len() != from.len() || to_orig.len() != to.len() {
        // Making it lower case changed how long it is, so the parts are compared one by one.
        let mut from_split: Vec<&[u16]> = from_orig.split(|unit| *unit == BACKSLASH).collect();
        let mut to_split: Vec<&[u16]> = to_orig.split(|unit| *unit == BACKSLASH).collect();
        if from_split.last().is_some_and(|last| last.is_empty()) {
            from_split.pop();
        }
        if to_split.last().is_some_and(|last| last.is_empty()) {
            to_split.pop();
        }
        let (from_len, to_len) = (from_split.len(), to_split.len());
        let length = from_len.min(to_len);
        let mut i = 0;
        while i < length && lower(from_split[i]) == lower(to_split[i]) {
            i += 1;
        }
        let join = |parts: &[&[u16]]| {
            let mut out = Vec::new();
            for (index, part) in parts.iter().enumerate() {
                if index > 0 {
                    out.push(BACKSLASH);
                }
                out.extend_from_slice(part);
            }
            out
        };
        let ups = |count: usize| -> Vec<u16> { (0..count).flat_map(|_| [DOT, DOT, BACKSLASH]).collect() };
        if i == 0 {
            return to_orig;
        } else if i == length {
            if to_len > length {
                return join(&to_split[i..]);
            }
            if from_len > length {
                let mut out = ups(from_len - 1 - i);
                out.extend_from_slice(&[DOT, DOT]);
                return out;
            }
            return Vec::new();
        }
        let mut out = ups(from_len - i);
        out.extend_from_slice(&join(&to_split[i..]));
        return out;
    }
    let mut from_start = 0;
    while from_start < from.len() && from[from_start] == BACKSLASH {
        from_start += 1;
    }
    let mut from_end = from.len();
    while from_end > from_start + 1 && from[from_end - 1] == BACKSLASH {
        from_end -= 1;
    }
    let from_len = from_end - from_start;
    let mut to_start = 0;
    while to_start < to.len() && to[to_start] == BACKSLASH {
        to_start += 1;
    }
    let mut to_end = to.len();
    while to_end > to_start + 1 && to[to_end - 1] == BACKSLASH {
        to_end -= 1;
    }
    let to_len = to_end - to_start;
    let length = from_len.min(to_len);
    let mut last_common_sep: isize = -1;
    let mut i = 0;
    while i < length {
        let from_code = from[from_start + i];
        if from_code != to[to_start + i] {
            break;
        } else if from_code == BACKSLASH {
            last_common_sep = i as isize;
        }
        i += 1;
    }
    if i != length {
        if last_common_sep == -1 {
            return to_orig;
        }
    } else {
        if to_len > length {
            if to[to_start + i] == BACKSLASH {
                // `from` is what `to` is in.
                return to_orig[to_start + i + 1..].to_vec();
            }
            if i == 2 {
                // `from` is the drive's root.
                return to_orig[to_start + i..].to_vec();
            }
        }
        if from_len > length {
            if from[from_start + i] == BACKSLASH {
                last_common_sep = i as isize;
            } else if i == 2 {
                last_common_sep = 3;
            }
        }
        if last_common_sep == -1 {
            last_common_sep = 0;
        }
    }
    let mut out: Vec<u16> = Vec::new();
    let mut index = from_start as isize + last_common_sep + 1;
    while index <= from_end as isize {
        if index == from_end as isize || from[index as usize] == BACKSLASH {
            if out.is_empty() {
                out.extend_from_slice(&[DOT, DOT]);
            } else {
                out.extend_from_slice(&[BACKSLASH, DOT, DOT]);
            }
        }
        index += 1;
    }
    let mut to_start = (to_start as isize + last_common_sep) as usize;
    if !out.is_empty() {
        out.extend_from_slice(slice(&to_orig, to_start as isize, to_end as isize));
        return out;
    }
    if at(&to_orig, to_start) == Some(BACKSLASH) {
        to_start += 1;
    }
    slice(&to_orig, to_start as isize, to_end as isize).to_vec()
}

/// A whole path in the form that has no limit on its length: `\\?\C:\...`, or `\\?\UNC\server\share\...`, or `None`
/// for what's given back as it is.
pub(super) fn to_namespaced_path(path: &[u16], ask: &mut Ask<'_>) -> Option<Vec<u16>> {
    if path.is_empty() {
        return None;
    }
    let resolved = resolve(&[path], ask);
    if resolved.len() <= 2 {
        return None;
    }
    if resolved[0] == BACKSLASH {
        if resolved[1] == BACKSLASH && resolved[2] != QUESTION && resolved[2] != DOT {
            let mut out = units("\\\\?\\UNC\\");
            out.extend_from_slice(&resolved[2..]);
            return Some(out);
        }
    } else if is_device_root(resolved[0]) && resolved[1] == COLON && resolved[2] == BACKSLASH {
        let mut out = units("\\\\?\\");
        out.extend_from_slice(&resolved);
        return Some(out);
    }
    Some(resolved)
}

/// How far the root at the start reaches, as `dirname` sees it, and where looking for the last part stops, or `Err`
/// with the whole answer when the path is only a network share.
fn dirname_root(path: &[u16]) -> Result<(isize, usize), ()> {
    let len = path.len();
    let code = path[0];
    if is_separator(code) {
        let mut root = (1, 1);
        if separator_at(path, 1) {
            if let Some((_, last, j)) = unc_parts(path) {
                if j == len {
                    return Err(());
                }
                if j != last {
                    root = ((j + 1) as isize, j + 1);
                }
            }
        }
        Ok(root)
    } else if is_device_root(code) && at(path, 1) == Some(COLON) {
        let end = if len > 2 && separator_at(path, 2) { 3 } else { 2 };
        Ok((end as isize, end))
    } else {
        Ok((-1, 0))
    }
}

pub(super) fn dirname(path: &[u16]) -> Vec<u16> {
    let len = path.len();
    if len == 0 {
        return vec![DOT];
    }
    if len == 1 {
        return if is_separator(path[0]) { path.to_vec() } else { vec![DOT] };
    }
    let Ok((root_end, offset)) = dirname_root(path) else {
        return path.to_vec();
    };
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut i = len as isize - 1;
    while i >= offset as isize {
        if is_separator(path[i as usize]) {
            if !matched_slash {
                end = i;
                break;
            }
        } else {
            matched_slash = false;
        }
        i -= 1;
    }
    if end == -1 {
        if root_end == -1 {
            return vec![DOT];
        }
        end = root_end;
    }
    path[..end as usize].to_vec()
}

pub(super) fn basename<'a>(path: &'a [u16], suffix: Option<&[u16]>) -> &'a [u16] {
    // What follows a drive letter isn't a separator at the end that can be left out.
    let start = if path.len() >= 2 && is_device_root(path[0]) && path[1] == COLON { 2 } else { 0 };
    basename_from(path, suffix, start, true)
}

/// The last part of a path, looking no further back than `start`, without `suffix` if it ends with that, for both kinds
/// of path.
pub(super) fn basename_from<'a>(path: &'a [u16], suffix: Option<&[u16]>, start: usize, windows: bool) -> &'a [u16] {
    let is_separator = |unit: u16| separates(unit, windows);
    let mut start = start;
    let mut end: isize = -1;
    let mut matched_slash = true;
    if let Some(suffix) = suffix.filter(|suffix| !suffix.is_empty() && suffix.len() <= path.len()) {
        if suffix == path {
            return &[];
        }
        let mut ext_index = suffix.len() as isize - 1;
        let mut first_non_slash_end: isize = -1;
        let mut i = path.len() as isize - 1;
        while i >= start as isize {
            let code = path[i as usize];
            if is_separator(code) {
                if !matched_slash {
                    start = (i + 1) as usize;
                    break;
                }
            } else {
                if first_non_slash_end == -1 {
                    matched_slash = false;
                    first_non_slash_end = i + 1;
                }
                if ext_index >= 0 {
                    if code == suffix[ext_index as usize] {
                        ext_index -= 1;
                        if ext_index == -1 {
                            end = i;
                        }
                    } else {
                        ext_index = -1;
                        end = first_non_slash_end;
                    }
                }
            }
            i -= 1;
        }
        if start as isize == end {
            end = first_non_slash_end;
        } else if end == -1 {
            end = path.len() as isize;
        }
        return slice(path, start as isize, end);
    }
    let mut i = path.len() as isize - 1;
    while i >= start as isize {
        if is_separator(path[i as usize]) {
            if !matched_slash {
                start = (i + 1) as usize;
                break;
            }
        } else if end == -1 {
            matched_slash = false;
            end = i + 1;
        }
        i -= 1;
    }
    if end == -1 {
        return &[];
    }
    slice(path, start as isize, end)
}

/// Where the last part starts and ends, and where its extension starts, looking back as far as `from`, as `extname` and
/// `parse` both find them: `(start_part, start_dot, end)`, with `start_part` as given if no separator comes before the
/// part, the dot `None` if it has no extension, and the end `None` if there's no part at all.
pub(super) fn last_part(
    path: &[u16],
    from: usize,
    start_part: usize,
    windows: bool,
) -> (usize, Option<usize>, Option<usize>) {
    let is_separator = |unit: u16| separates(unit, windows);
    let root_end = from;
    let mut start_dot: isize = -1;
    let mut start_part = start_part;
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut pre_dot_state = 0;
    let mut i = path.len() as isize - 1;
    while i >= root_end as isize {
        let code = path[i as usize];
        if is_separator(code) {
            if !matched_slash {
                start_part = (i + 1) as usize;
                break;
            }
            i -= 1;
            continue;
        }
        if end == -1 {
            matched_slash = false;
            end = i + 1;
        }
        if code == DOT {
            if start_dot == -1 {
                start_dot = i;
            } else if pre_dot_state != 1 {
                pre_dot_state = 1;
            }
        } else if start_dot != -1 {
            pre_dot_state = -1;
        }
        i -= 1;
    }
    if end == -1 {
        return (start_part, None, None);
    }
    let no_extension = start_dot == -1
        || pre_dot_state == 0
        // The last part is `..`.
        || (pre_dot_state == 1 && start_dot == end - 1 && start_dot == start_part as isize + 1);
    let dot = if no_extension { None } else { Some(start_dot as usize) };
    (start_part, dot, Some(end as usize))
}

pub(super) fn extname(path: &[u16]) -> &[u16] {
    let start = if path.len() >= 2 && path[1] == COLON && is_device_root(path[0]) { 2 } else { 0 };
    match last_part(path, start, start, true) {
        (_, Some(dot), Some(end)) => &path[dot..end],
        _ => &[],
    }
}

/// `parse`'s root, dir, base, ext and name.
pub(super) fn parse(path: &[u16]) -> [Vec<u16>; 5] {
    let mut root: &[u16] = &[];
    let len = path.len();
    if len == 0 {
        return Default::default();
    }
    let code = path[0];
    if len == 1 {
        return if is_separator(code) {
            [path.to_vec(), path.to_vec(), Vec::new(), Vec::new(), Vec::new()]
        } else {
            [Vec::new(), Vec::new(), path.to_vec(), Vec::new(), path.to_vec()]
        };
    }
    let mut root_end = 0;
    if is_separator(code) {
        root_end = 1;
        if separator_at(path, 1) {
            if let Some((_, last, j)) = unc_parts(path) {
                if j == len {
                    root_end = j;
                } else if j != last {
                    root_end = j + 1;
                }
            }
        }
    } else if is_device_root(code) && path[1] == COLON {
        if len <= 2 {
            return [path.to_vec(), path.to_vec(), Vec::new(), Vec::new(), Vec::new()];
        }
        root_end = 2;
        if separator_at(path, 2) {
            if len == 3 {
                return [path.to_vec(), path.to_vec(), Vec::new(), Vec::new(), Vec::new()];
            }
            root_end = 3;
        }
    }
    if root_end > 0 {
        root = &path[..root_end];
    }
    let (start_part, dot, end) = last_part(path, root_end, root_end, true);
    let (mut base, mut ext, mut name): (&[u16], &[u16], &[u16]) = (&[], &[], &[]);
    if let Some(end) = end {
        base = &path[start_part..end];
        match dot {
            Some(dot) => {
                name = &path[start_part..dot];
                ext = &path[dot..end];
            }
            None => name = base,
        }
    }
    // A folder that's the root is all of the root, with its separator, and any other is without the one at its end.
    let dir = if start_part > 0 && start_part != root_end { &path[..start_part - 1] } else { root };
    [root.to_vec(), dir.to_vec(), base.to_vec(), ext.to_vec(), name.to_vec()]
}
