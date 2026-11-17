//! CSS grid, for what's easier to say as rows and columns than as boxes inside boxes. It's kept apart from flexbox, and only a
//! grid reads any of this, so nothing else pays for it.

/// How big a row or a column is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Track {
    Px(f32),
    /// A percentage of the grid's size, if that's known.
    Percent(f32),
    /// A share of what's left.
    Fr(f32),
    /// As big as what's in it.
    Auto,
    /// At least this many pixels, and as big as the most it can be says, as CSS's `minmax()` has it.
    MinMax(f32, Most),
}

/// The most a track made with `minmax()` can be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Most {
    /// As big as what's in it.
    Auto,
    /// As big as what's in it, up to this many pixels.
    Px(f32),
    /// As big as what's in it, up to this share of the grid's size.
    Percent(f32),
    /// This share of what's left, however big what's in it is.
    Fr(f32),
}

impl Track {
    /// Whether how big it is depends on what's in it.
    pub(crate) fn by_content(self) -> bool {
        matches!(
            self,
            Track::Auto | Track::MinMax(_, Most::Auto | Most::Px(_) | Most::Percent(_))
        )
    }

    /// Whether it takes a share of the room nothing else uses, as `auto` does when a grid is stretched.
    pub(crate) fn stretches(self) -> bool {
        matches!(self, Track::Auto | Track::MinMax(_, Most::Auto))
    }
}

/// Where a child goes: which track it starts on, if it says, and how many it takes up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) start: Option<usize>,
    pub(crate) count: usize,
}

impl Default for Span {
    fn default() -> Self {
        Self {
            start: None,
            count: 1,
        }
    }
}

fn one_track(text: &str) -> Track {
    let text = text.trim();
    if let Some(number) = text.strip_suffix("fr") {
        return Track::Fr(number.trim().parse().unwrap_or(1.0));
    }
    if let Some(number) = text.strip_suffix('%') {
        return Track::Percent(number.trim().parse().unwrap_or(0.0));
    }
    if let Some(number) = text.strip_suffix("px") {
        return Track::Px(number.trim().parse().unwrap_or(0.0));
    }
    if let Some(inside) = text
        .strip_prefix("minmax(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts = split_top_level(inside, ',');
        let least = parts.first().map_or(0.0, |first| least_of(first));
        let most = match parts.get(1).map(|last| one_track(last)) {
            Some(Track::Px(pixels)) => Most::Px(pixels),
            Some(Track::Percent(percent)) => Most::Percent(percent),
            Some(Track::Fr(share)) => Most::Fr(share),
            _ => Most::Auto,
        };
        return Track::MinMax(least, most);
    }
    match text.parse::<f32>() {
        Ok(number) => Track::Px(number),
        // `auto`, `min-content`, `max-content` and `fit-content()` all come to what's in it.
        Err(_) => Track::Auto,
    }
}

/// Splits text on this, other than inside brackets.
fn split_top_level(text: &str, on: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut depth = 0;
    for character in text.chars() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && (character == on || (on == ' ' && character.is_whitespace())) {
            if !part.trim().is_empty() {
                parts.push(part.trim().to_owned());
            }
            part.clear();
        } else {
            part.push(character);
        }
    }
    if !part.trim().is_empty() {
        parts.push(part.trim().to_owned());
    }
    parts
}

/// Reads tracks as CSS writes them: `1fr 200px auto`, `repeat(3, 1fr)` or `minmax(100px, 1fr) 50%`.
pub(crate) fn tracks(text: &str) -> Vec<Track> {
    tracks_in(text, None, 0.0, usize::MAX)
}

/// The least a track can be, in pixels, as `minmax(70px, 1fr)` or `70px` says, or 0 if it doesn't say.
fn least_of(text: &str) -> f32 {
    let text = text.trim();
    if let Some(inside) = text
        .strip_prefix("minmax(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return split_top_level(inside, ',')
            .first()
            .map_or(0.0, |first| least_of(first));
    }
    match one_track(text) {
        Track::Px(pixels) => pixels,
        _ => 0.0,
    }
}

/// Like [`tracks`], with `repeat(auto-fill, ...)` and `repeat(auto-fit, ...)` as many times as fit in `room` with `gap` between,
/// as CSS has them. With `auto-fit`, there are no more than `items`, so that what there is fills the room.
pub(crate) fn tracks_in(text: &str, room: Option<f32>, gap: f32, items: usize) -> Vec<Track> {
    let mut out = Vec::new();
    for part in split_top_level(text, ' ') {
        if let Some(inside) = part
            .strip_prefix("repeat(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            let parts = split_top_level(inside, ',');
            if let [count, what] = parts.as_slice() {
                // How many fit depends on what's in them, which a window this big rarely wants a lot of.
                let count: usize = match count.as_str() {
                    kind @ ("auto-fill" | "auto-fit") => {
                        let each: f32 = split_top_level(what, ' ')
                            .iter()
                            .map(|one| least_of(one) + gap)
                            .sum::<f32>()
                            // A track that could be nothing at all, as `minmax(0, 1fr)` is, counts as a pixel, as browsers have
                            // it, so that it fits a number of times rather than without end.
                            .max(1.0);
                        let fit = match room {
                            Some(room) if room.is_finite() => {
                                ((room + gap) / each).floor().max(1.0) as usize
                            }
                            _ => 1,
                        };
                        if kind == "auto-fit" {
                            fit.min(items.max(1))
                        } else {
                            fit
                        }
                    }
                    number => number.parse().unwrap_or(1),
                };
                let what = tracks(what);
                for _ in 0..count.min(1000) {
                    out.extend_from_slice(&what);
                }
            }
        } else {
            out.push(one_track(&part));
        }
    }
    out
}

/// Reads where something goes as CSS writes it: `2`, `span 2`, `1 / 3`, `2 / span 2` or `1 / -1`, with `columns` being how
/// many tracks there are to count back from.
/// The areas a grid names, as CSS's `grid-template-areas` writes them: a quoted row of names for each row, with `.` for a cell
/// with no name. Each name covers the rectangle of cells it's in.
#[derive(Debug, Default)]
pub(crate) struct Areas {
    pub(crate) rows: usize,
    pub(crate) columns: usize,
    /// Each name, with the first and last row and column it's in, counting from 0.
    named: Vec<(String, [usize; 4])>,
}

impl Areas {
    /// Where an area is, as `grid_column` and `grid_row` would say it.
    pub(crate) fn find(&self, name: &str) -> Option<(String, String)> {
        let name = name.trim();
        let (_, [top, left, bottom, right]) = self.named.iter().find(|(said, _)| said == name)?;
        Some((
            format!("{} / {}", left + 1, right + 2),
            format!("{} / {}", top + 1, bottom + 2),
        ))
    }
}

/// Reads `grid-template-areas`.
pub(crate) fn areas(text: &str) -> Areas {
    let mut found = Areas::default();
    let rows: Vec<Vec<&str>> = text
        .split(['"', '\''])
        .enumerate()
        // What's inside quotes is every other piece.
        .filter(|(at, _)| at % 2 == 1)
        .map(|(_, row)| row.split_whitespace().collect())
        .filter(|row: &Vec<&str>| !row.is_empty())
        .collect();
    found.rows = rows.len();
    found.columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    for (row, names) in rows.iter().enumerate() {
        for (column, name) in names.iter().enumerate() {
            if name.chars().all(|c| c == '.') {
                continue;
            }
            match found.named.iter_mut().find(|(said, _)| said == name) {
                Some((_, place)) => {
                    place[0] = place[0].min(row);
                    place[1] = place[1].min(column);
                    place[2] = place[2].max(row);
                    place[3] = place[3].max(column);
                }
                None => found
                    .named
                    .push(((*name).to_owned(), [row, column, row, column])),
            }
        }
    }
    found
}

/// Where something goes along one way, as `grid-column` or `grid-row` says it: where it starts, and where it ends after a `/`.
pub(crate) fn span(text: &str, count: usize) -> Span {
    let parts = split_top_level(text, '/');
    match parts.as_slice() {
        [one] => span_between(one, "auto", count),
        [first, second] => span_between(first, second, count),
        _ => Span::default(),
    }
}

/// One side of where something goes in a grid.
#[derive(Clone, Copy)]
enum Edge {
    /// Wherever there's room, as `auto` and a line's name, which isn't anything here, are.
    Auto,
    /// A line, from 1 at the start, and from -1 at the end.
    Line(i64),
    /// As many tracks from the other side, as `span 2` says.
    Span(usize),
}

fn edge(text: &str) -> Edge {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("span") {
        return rest
            .trim()
            .parse::<usize>()
            .map_or(Edge::Span(1), |count| Edge::Span(count.max(1)));
    }
    match text.parse::<i64>() {
        Ok(0) | Err(_) => Edge::Auto,
        Ok(number) => Edge::Line(number),
    }
}

/// Where something goes along one way, from where it starts and where it ends, as CSS's `grid-column-start` and `grid-column-end`
/// say, each of which can be `auto`, a line, or `span` and how many tracks, out of `count`, as CSS has them.
pub(crate) fn span_between(start: &str, end: &str, count: usize) -> Span {
    // A line, from 1 at the start, and from -1 at the end, which is one further along than the track it starts.
    let line = |number: i64| -> usize {
        if number < 0 {
            (count as i64 + 2 + number).max(1) as usize
        } else {
            number.max(1) as usize
        }
    };
    let (start_line, count) = match (edge(start), edge(end)) {
        (Edge::Line(from), Edge::Line(to)) => {
            let (from, to) = (line(from), line(to));
            // Lines the wrong way around are the other way around, and the same line twice is one track, as CSS has them.
            let (from, to) = (from.min(to), from.max(to));
            (Some(from), (to - from).max(1))
        }
        (Edge::Line(from), Edge::Span(tracks)) => (Some(line(from)), tracks),
        (Edge::Line(from), Edge::Auto) => (Some(line(from)), 1),
        (Edge::Span(tracks), Edge::Line(to)) => {
            let to = line(to);
            (Some(to.saturating_sub(tracks).max(1)), tracks)
        }
        (Edge::Auto, Edge::Line(to)) => (Some(line(to).saturating_sub(1).max(1)), 1),
        (Edge::Span(tracks), _) | (Edge::Auto, Edge::Span(tracks)) => (None, tracks),
        (Edge::Auto, Edge::Auto) => (None, 1),
    };
    Span {
        start: start_line.map(|line| line - 1),
        count: count.max(1),
    }
}

/// Where each child goes, in `(column, row)`, given where each says it wants to be. What says both its column and its row goes
/// there first, and what doesn't say goes in the next place with room after that, a row at a time, as CSS does: shadcn's
/// CardAction says column 2 rows 1 and 2, and the title and description after it go under each other in column 1. It says how many
/// rows that came to.
pub(crate) fn place(wanted: &[(Span, Span)], columns: usize) -> (Vec<(usize, usize)>, usize) {
    let mut taken: Vec<Vec<bool>> = Vec::new();
    let mut places = Vec::with_capacity(wanted.len());
    let mut fits = |column: usize, row: usize, across: usize, down: usize, mark: bool| -> bool {
        for at_row in row..row + down {
            if taken.len() <= at_row {
                taken.resize(at_row + 1, vec![false; columns]);
            }
            let end = (column + across).min(columns);
            for cell in &mut taken[at_row][column..end] {
                if *cell {
                    return false;
                }
                if mark {
                    *cell = true;
                }
            }
        }
        true
    };
    let (mut next_column, mut next_row) = (0, 0);
    let mut most_rows = 0;
    let mut fixed = vec![None; wanted.len()];
    for (at, (across, down)) in wanted.iter().enumerate() {
        if let (Some(column), Some(row)) = (across.start, down.start) {
            let place = (column.min(columns - 1), row);
            fits(place.0, place.1, across.count.min(columns), down.count, true);
            fixed[at] = Some(place);
        }
    }
    for (at, (across, down)) in wanted.iter().enumerate() {
        let width = across.count.min(columns);
        if let Some(place) = fixed[at] {
            most_rows = most_rows.max(place.1 + down.count);
            places.push(place);
            continue;
        }
        let place = match (across.start, down.start) {
            (Some(column), Some(row)) => (column.min(columns - 1), row),
            (Some(column), None) => {
                let column = column.min(columns - 1);
                let mut row = next_row;
                while !fits(column, row, width, down.count, false) {
                    row += 1;
                }
                (column, row)
            }
            (None, Some(row)) => {
                let mut column = 0;
                while column + width < columns && !fits(column, row, width, down.count, false) {
                    column += 1;
                }
                (column, row)
            }
            (None, None) => {
                loop {
                    if next_column + width > columns {
                        next_column = 0;
                        next_row += 1;
                    }
                    if fits(next_column, next_row, width, down.count, false) {
                        break;
                    }
                    next_column += 1;
                }
                let place = (next_column, next_row);
                next_column += width;
                place
            }
        };
        fits(place.0, place.1, width, down.count, true);
        most_rows = most_rows.max(place.1 + down.count);
        places.push(place);
    }
    (places, most_rows)
}

/// How big each track is. `wanted` is what's in each that's as big as what's in it, and `room` is how much there is altogether if
/// that's known. Without that, a share comes to what its contents want. With `stretch`, tracks sized by what's in them share out what's left, as CSS's grids do when there's nothing
/// to say otherwise.
pub(crate) fn sizes(
    tracks: &[Track],
    wanted: &[f32],
    room: Option<f32>,
    gap: f32,
    stretch: bool,
) -> Vec<f32> {
    let mut out = vec![0f32; tracks.len()];
    // The share each flexible track takes, and the least it can be, which a share smaller than that doesn't get it below.
    let mut flexible: Vec<(usize, f32, f32)> = Vec::new();
    let mut most_per_share = 0f32;
    for (at, track) in tracks.iter().enumerate() {
        out[at] = match track {
            Track::Px(size) => *size,
            Track::Percent(percent) => room.map_or(wanted[at], |room| room * percent / 100.0),
            Track::Auto => wanted[at],
            Track::MinMax(least, Most::Auto) => wanted[at].max(*least),
            // With room to grow into, a track grows to the most it can be before what's left is shared out, as CSS has it.
            Track::MinMax(least, Most::Px(most)) => if room.is_some() {
                *most
            } else {
                wanted[at].min(*most)
            }
            .max(*least),
            Track::MinMax(least, Most::Percent(percent)) => match room {
                Some(room) => (room * percent / 100.0).max(*least),
                None => wanted[at].max(*least),
            },
            Track::Fr(share) | Track::MinMax(_, Most::Fr(share)) => {
                let least = match track {
                    Track::MinMax(least, _) => *least,
                    _ => 0.0,
                };
                flexible.push((at, *share, least));
                if *share > 0.0 {
                    most_per_share = most_per_share.max(wanted[at].max(least) / share);
                }
                least
            }
        };
    }
    if !flexible.is_empty() {
        match room {
            Some(room) => {
                // What's left is shared out, and a track whose share would be less than its least is that instead, which
                // leaves less for the others, as CSS has it.
                let mut fixed = vec![false; flexible.len()];
                loop {
                    let used: f32 = out
                        .iter()
                        .enumerate()
                        .filter(|(at, _)| {
                            !flexible
                                .iter()
                                .zip(&fixed)
                                .any(|((one, _, _), fixed)| one == at && !fixed)
                        })
                        .map(|(_, size)| size)
                        .sum::<f32>()
                        + gap * tracks.len().saturating_sub(1) as f32;
                    let shares: f32 = flexible
                        .iter()
                        .zip(&fixed)
                        .filter(|(_, fixed)| !**fixed)
                        .map(|((_, share, _), _)| share)
                        .sum();
                    let per_share = if shares > 0.0 {
                        ((room - used) / shares).max(0.0)
                    } else {
                        0.0
                    };
                    let mut changed = false;
                    for ((at, share, least), fixed) in flexible.iter().zip(fixed.iter_mut()) {
                        if *fixed {
                            continue;
                        }
                        if per_share * share < *least {
                            *fixed = true;
                            out[*at] = *least;
                            changed = true;
                        } else {
                            out[*at] = per_share * share;
                        }
                    }
                    if !changed {
                        break;
                    }
                }
            }
            None => {
                for (at, share, least) in &flexible {
                    out[*at] = (most_per_share * share).max(*least);
                }
            }
        }
    } else if let (true, Some(room)) = (stretch, room) {
        let used: f32 = out.iter().sum::<f32>() + gap * tracks.len().saturating_sub(1) as f32;
        let autos = tracks.iter().filter(|track| track.stretches()).count();
        if used < room && autos > 0 {
            let each = (room - used) / autos as f32;
            for (at, track) in tracks.iter().enumerate() {
                if track.stretches() {
                    out[at] += each;
                }
            }
        }
    }
    out
}

/// How tracks share out the room they don't fill, as CSS's `justify-content` and `align-content` say.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Spread {
    Start,
    Center,
    End,
    Between,
    Around,
    Evenly,
}

impl Spread {
    /// What a word for it means, which is the start for anything else.
    pub(crate) fn read(word: &str) -> Self {
        match word {
            "center" => Self::Center,
            "end" | "flex-end" => Self::End,
            "space_between" | "space-between" => Self::Between,
            "space_around" | "space-around" => Self::Around,
            "space_evenly" | "space-evenly" => Self::Evenly,
            _ => Self::Start,
        }
    }

    /// Where the first track starts, from the start of the room, and how much more goes between each two of them, with `free` the room
    /// left over. Spaces between them need room to share, and without it they start at the start, as CSS has it.
    pub(crate) fn offsets(self, free: f32, count: usize) -> (f32, f32) {
        let count = count as f32;
        match self {
            Self::Start => (0.0, 0.0),
            Self::Center => (free / 2.0, 0.0),
            Self::End => (free, 0.0),
            _ if free <= 0.0 || count == 0.0 => (0.0, 0.0),
            Self::Between if count < 2.0 => (0.0, 0.0),
            Self::Between => (0.0, free / (count - 1.0)),
            Self::Around => (free / count / 2.0, free / count),
            Self::Evenly => (free / (count + 1.0), free / (count + 1.0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_left_over_is_shared_out_as_css_says() {
        assert_eq!(Spread::Start.offsets(30.0, 3), (0.0, 0.0));
        assert_eq!(Spread::Center.offsets(30.0, 3), (15.0, 0.0));
        assert_eq!(Spread::End.offsets(30.0, 3), (30.0, 0.0));
        assert_eq!(Spread::Between.offsets(30.0, 3), (0.0, 15.0));
        assert_eq!(Spread::Around.offsets(30.0, 3), (5.0, 10.0));
        assert_eq!(Spread::Evenly.offsets(30.0, 2), (10.0, 10.0));
        assert_eq!(Spread::Between.offsets(30.0, 1), (0.0, 0.0));
        assert_eq!(Spread::Evenly.offsets(-10.0, 2), (0.0, 0.0));
        assert_eq!(Spread::Center.offsets(-10.0, 2), (-5.0, 0.0));
    }

    #[test]
    fn tracks_are_read_as_css_writes_them() {
        assert_eq!(
            tracks("1fr 200px 50% auto 30"),
            [
                Track::Fr(1.0),
                Track::Px(200.0),
                Track::Percent(50.0),
                Track::Auto,
                Track::Px(30.0)
            ]
        );
        assert_eq!(
            tracks("repeat(2, 1fr) 10px"),
            [Track::Fr(1.0), Track::Fr(1.0), Track::Px(10.0)]
        );
        assert_eq!(tracks("repeat(2, 1fr 10px)").len(), 4);
        assert_eq!(
            tracks("minmax(100px, 1fr) minmax(50px, auto) max-content"),
            [
                Track::MinMax(100.0, Most::Fr(1.0)),
                Track::MinMax(50.0, Most::Auto),
                Track::Auto
            ]
        );
    }

    #[test]
    fn where_something_goes_is_read_as_css_writes_it() {
        let at = |text: &str| span(text, 3);
        assert_eq!(
            at("2"),
            Span {
                start: Some(1),
                count: 1
            }
        );
        assert_eq!(
            at("span 2"),
            Span {
                start: None,
                count: 2
            }
        );
        assert_eq!(
            at("1 / 3"),
            Span {
                start: Some(0),
                count: 2
            }
        );
        assert_eq!(
            at("2 / span 2"),
            Span {
                start: Some(1),
                count: 2
            }
        );
        // From the first line to the last, in three columns.
        assert_eq!(
            at("1 / -1"),
            Span {
                start: Some(0),
                count: 3
            }
        );
        // As many before where it ends, the track before where it ends, lines the wrong way around, and either side by itself.
        let place = |start: Option<usize>, count: usize| Span { start, count };
        assert_eq!(at("span 2 / 4"), place(Some(1), 2));
        assert_eq!(at("auto / 3"), place(Some(1), 1));
        assert_eq!(at("3 / 1"), place(Some(0), 2));
        assert_eq!(at("auto / span 3"), place(None, 3));
        assert_eq!(span_between("2", "auto", 3), place(Some(1), 1));
        assert_eq!(span_between("auto", "-1", 3), place(Some(2), 1));
        assert_eq!(span_between("span 2", "span 2", 3), place(None, 2));
    }

    #[test]
    fn what_does_not_say_goes_in_the_next_place_with_room() {
        let free = (Span::default(), Span::default());
        let wide = (
            Span {
                start: None,
                count: 2,
            },
            Span::default(),
        );
        let (places, rows) = place(&[free, wide, free], 2);
        assert_eq!(places, [(0, 0), (0, 1), (0, 2)]);
        assert_eq!(rows, 3);
        let tall = (
            Span::default(),
            Span {
                start: None,
                count: 2,
            },
        );
        let (places, rows) = place(&[tall, free, free], 2);
        assert_eq!(places, [(0, 0), (1, 0), (1, 1)]);
        assert_eq!(rows, 2);
        let there = (
            Span {
                start: Some(1),
                count: 1,
            },
            Span {
                start: Some(1),
                count: 1,
            },
        );
        let (places, _) = place(&[there, free], 2);
        assert_eq!(places, [(1, 1), (0, 0)]);
        // What says both where it goes is put there before what comes before it goes in the next place with room, as
        // shadcn's CardAction (column 2, rows 1 and 2) after its title and description.
        let action = (
            Span {
                start: Some(1),
                count: 1,
            },
            Span {
                start: Some(0),
                count: 2,
            },
        );
        let (places, rows) = place(&[free, free, action], 2);
        assert_eq!(places, [(0, 0), (0, 1), (1, 0)]);
        assert_eq!(rows, 2);
    }

    #[test]
    fn something_too_wide_for_the_grid_still_finds_a_place() {
        let wide = (
            Span {
                start: None,
                count: 9,
            },
            Span::default(),
        );
        let (places, rows) = place(&[wide, wide], 2);
        assert_eq!(places, [(0, 0), (0, 1)]);
        assert_eq!(rows, 2);
    }

    #[test]
    fn auto_fit_makes_as_many_columns_as_fit() {
        // 70 each and 6 between: three fit in 288, as (288 + 6) / 76 is 3 and some.
        assert_eq!(
            tracks_in("repeat(auto-fit, minmax(70px, 1fr))", Some(288.0), 6.0, 7).len(),
            3
        );
        // With only two things, auto-fit makes two, so they fill it, and auto-fill makes as many as fit anyway.
        assert_eq!(
            tracks_in("repeat(auto-fit, minmax(70px, 1fr))", Some(288.0), 6.0, 2).len(),
            2
        );
        assert_eq!(
            tracks_in("repeat(auto-fill, 70px)", Some(288.0), 6.0, 2).len(),
            3
        );
        // Without knowing the room, there's one.
        assert_eq!(tracks("repeat(auto-fill, 70px)").len(), 1);
    }

    #[test]
    fn a_track_that_could_be_nothing_repeats_as_a_pixel_would() {
        // Pico's `.grid`: one column for each thing in it, as many of them fit.
        assert_eq!(
            tracks_in("repeat(auto-fit, minmax(0%, 1fr))", Some(700.0), 0.0, 3).len(),
            3
        );
        assert_eq!(
            tracks_in("repeat(auto-fit, minmax(0, 1fr))", Some(700.0), 0.0, 3).len(),
            3
        );
        // And auto-fill makes one a pixel wide for each pixel there is, as browsers do.
        assert_eq!(
            tracks_in("repeat(auto-fill, minmax(0, 1fr))", Some(300.0), 0.0, 3).len(),
            300
        );
    }

    #[test]
    fn what_is_left_is_shared_out() {
        let tracks = [
            Track::Px(100.0),
            Track::Fr(1.0),
            Track::Fr(3.0),
            Track::Auto,
        ];
        let got = sizes(&tracks, &[0.0, 0.0, 0.0, 40.0], Some(380.0), 10.0, false);
        // 380 - 100 - 40 - 30 for gaps is 210, in four shares.
        assert_eq!(got, [100.0, 52.5, 157.5, 40.0]);
        assert_eq!(
            sizes(&[Track::Percent(25.0)], &[9.0], Some(200.0), 0.0, false),
            [50.0]
        );
    }

    #[test]
    fn with_no_room_known_a_share_is_what_its_contents_want() {
        let tracks = [Track::Fr(1.0), Track::Fr(2.0)];
        // The first wants 30 for one share, and the second 20 for two, so each share has to be 30.
        assert_eq!(
            sizes(&tracks, &[30.0, 20.0], None, 0.0, false),
            [30.0, 60.0]
        );
    }

    #[test]
    fn too_little_room_leaves_shares_with_nothing_rather_than_less() {
        assert_eq!(
            sizes(
                &[Track::Px(100.0), Track::Fr(1.0)],
                &[0.0, 0.0],
                Some(50.0),
                0.0,
                false
            ),
            [100.0, 0.0]
        );
    }

    #[test]
    fn tracks_sized_by_what_is_in_them_share_what_is_left_as_css_has_them() {
        let tracks = [Track::Auto, Track::Px(10.0), Track::Auto];
        assert_eq!(
            sizes(&tracks, &[20.0, 0.0, 30.0], Some(100.0), 0.0, true),
            [40.0, 10.0, 50.0]
        );
        assert_eq!(
            sizes(&tracks, &[20.0, 0.0, 30.0], Some(100.0), 0.0, false),
            [20.0, 10.0, 30.0]
        );
    }
}
