//! Tables, as CSS lays them out: rows of cells, where each column is as wide as what's widest in it in any row, and each row
//! as tall as what's tallest in it. Rows can be in groups, as `<tbody>` holds them, and what's in a table, a group or a row
//! that isn't what goes there is in a cell of its own, as CSS makes one up around it. A row or a group with no table around
//! it is a table by itself.
//!
//! Only cells are laid out by themselves. Rows and groups are as big as the cells in them say, and are never asked
//! anything, so they never keep an answer of their own: when one changes, what's around it is told it may need asking
//! again, which comes back to the table.

use rsui_core::{Element, PropValue};

use super::{
    Axis, ClampTo, Key, Mode, Pass, asked_about, mark_overlay, padding_totals, resolve_auto,
    resolve_length,
};
use crate::measure::Measure;
use crate::style::{NAMES, Style, TablePart};
use crate::tree::{
    CHILD_DIRTY, IS_ABSOLUTE, IS_OVERLAY, LayoutBox, MANY_CHILDREN_DIRTY, SELF_DIRTY, Size,
};

/// Where something is from the table, as the index of each step down to it: as many of `steps` as `len` says.
#[derive(Clone, Copy)]
struct Path {
    steps: [usize; 3],
    len: usize,
}

impl Path {
    const HERE: Self = Self {
        steps: [0; 3],
        len: 0,
    };

    fn then(self, index: usize) -> Self {
        let mut out = self;
        out.steps[out.len] = index;
        out.len += 1;
        out
    }

    fn steps(&self) -> &[usize] {
        &self.steps[..self.len]
    }
}

/// Whether a table's caption goes below its rows, as `caption-side: bottom` says, on the caption or on the table, which it
/// inherits from, as CSS has it.
fn below_rows(table: &Element, caption: &Element) -> bool {
    let side = caption.props.get("caption_side").or_else(|| table.props.get("caption_side"));
    matches!(side, Some(PropValue::Str(side)) if side.as_str() == "bottom")
}

fn box_at(mut at: &mut LayoutBox, path: Path) -> &mut LayoutBox {
    for &index in path.steps() {
        at = &mut at.children[index];
    }
    at
}

fn node_at(mut at: &Element, path: Path) -> &Element {
    for &index in path.steps() {
        at = &at.children[index];
    }
    at
}

struct Cell {
    at: Path,
    style: Style,
    row: usize,
    column: usize,
    across: usize,
    down: usize,
    /// Its margins, top, right, bottom and left, when it's something that isn't a cell where a cell goes, which is in a cell
    /// of its own, as CSS makes one around it, so its margins are inside that: a calendar's week, as a `<tr>` that's
    /// `display: flex`, with a margin above it (react-day-picker). A cell itself has none.
    margins: [f32; 4],
}

impl Cell {
    fn margin_x(&self) -> f32 {
        self.margins[1] + self.margins[3]
    }
    fn margin_y(&self) -> f32 {
        self.margins[0] + self.margins[2]
    }
}

struct Row {
    /// Where the row is, unless it's made up around something that was by itself where a row goes.
    at: Option<Path>,
    group: Option<usize>,
    /// How tall it says it is at least.
    height: f32,
}

struct Group {
    at: Path,
    /// Its rows, from the first to the one after the last.
    rows: (usize, usize),
}

/// What's in a table, worked out from the tree.
#[derive(Default)]
struct Parts {
    rows: Vec<Row>,
    groups: Vec<Group>,
    cells: Vec<Cell>,
    captions: Vec<(Path, Style)>,
    /// How wide each column says it is, as a `<col>` does.
    said_widths: Vec<Option<f32>>,
    /// What has something in it that's on top of everything, or placed by itself, which is laid out once the rest is.
    overlays: Vec<Path>,
    absolutes: Vec<Path>,
}

/// A whole number from a prop, as a cell says how many columns or rows it goes across.
fn count_of(node: &Element, name: rsui_core::Symbol) -> usize {
    match node.props.get_symbol(name) {
        Some(PropValue::Int(count)) => (*count).max(0) as usize,
        Some(PropValue::Float(count)) if count.is_finite() => count.max(0.0) as usize,
        _ => 1,
    }
}

impl Parts {
    /// Notes what's on top of everything, or placed by itself, among what's in this, and says whether `child` is either.
    fn set_aside(
        &mut self,
        child_box: &mut LayoutBox,
        child: &Element,
        style: &Style,
        parent: Path,
    ) -> bool {
        if style.overlay.is_some() {
            mark_overlay(child_box, child);
            if !self
                .overlays
                .iter()
                .any(|one| one.steps() == parent.steps())
            {
                self.overlays.push(parent);
            }
            return true;
        }
        child_box.flags &= !IS_OVERLAY;
        if style.absolute {
            child_box.flags |= IS_ABSOLUTE;
            crate::engine::mark_absolute_inert(child_box, child);
            if !self
                .absolutes
                .iter()
                .any(|one| one.steps() == parent.steps())
            {
                self.absolutes.push(parent);
            }
            return true;
        }
        child_box.flags &= !IS_ABSOLUTE;
        false
    }

    /// What's where a row goes, at `at`: a row, with each thing in it a cell, or anything else, which is a cell in a row of its own.
    fn add_row(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        at: Path,
        style: Style,
        group: Option<usize>,
    ) {
        let row = self.rows.len();
        if style.table != TablePart::Row {
            self.rows.push(Row {
                at: None,
                group,
                height: 0.0,
            });
            self.add_cell(node_at(node, at), at, style, row);
            return;
        }
        let height = resolve_length(style.height, None)
            .unwrap_or(0.0)
            .max(style.min_height);
        self.rows.push(Row {
            at: Some(at),
            group,
            height,
        });
        let row_node = node_at(node, at);
        let row_box = box_at(current, at);
        if row_box.children.len() != row_node.children.len() {
            super::set_box_count(row_box, row_node.children.len());
        }
        for (index, child) in row_node.children.iter().enumerate() {
            let (child_style, _) = Style::read(&child.props);
            if self.set_aside(
                &mut box_at(current, at).children[index],
                child,
                &child_style,
                at,
            ) {
                continue;
            }
            self.add_cell(child, at.then(index), child_style, row);
        }
    }

    fn add_cell(&mut self, node: &Element, at: Path, style: Style, row: usize) {
        let names = &*NAMES;
        let margins = if style.table != TablePart::Cell && style.has_margin {
            Style::read_margins(&node.props)
        } else {
            [0.0; 4]
        };
        self.cells.push(Cell {
            margins,
            at,
            style,
            row,
            column: 0,
            across: count_of(node, names.col_span).max(1),
            down: count_of(node, names.row_span),
        });
    }

    /// What a `<col>` or a `<colgroup>` says about how wide its columns are.
    fn add_columns(&mut self, node: &Element, style: &Style) {
        let names = &*NAMES;
        if style.table == TablePart::Columns && !node.children.is_empty() {
            for child in &node.children {
                let (child_style, _) = Style::read(&child.props);
                if child_style.table == TablePart::Column {
                    self.add_columns(child, &child_style);
                }
            }
            return;
        }
        let width = resolve_length(style.width, None);
        for _ in 0..count_of(node, names.col_span).max(1) {
            self.said_widths.push(width);
        }
    }

    /// Finds the rows and cells of `node`, which is a table, a group or a row.
    fn find(current: &mut LayoutBox, node: &Element, own: TablePart) -> Self {
        let mut parts = Parts::default();
        if own == TablePart::Row {
            let (style, _) = Style::read(&node.props);
            parts.add_row(current, node, Path::HERE, style, None);
            return parts;
        }
        for (index, child) in node.children.iter().enumerate() {
            let (style, _) = Style::read(&child.props);
            if parts.set_aside(&mut current.children[index], child, &style, Path::HERE) {
                continue;
            }
            let at = Path::HERE.then(index);
            match style.table {
                TablePart::Group if own == TablePart::Table => {
                    let first = parts.rows.len();
                    let group = parts.groups.len();
                    parts.groups.push(Group {
                        at,
                        rows: (first, first),
                    });
                    let group_box = &mut current.children[index];
                    if group_box.children.len() != child.children.len() {
                        super::set_box_count(group_box, child.children.len());
                    }
                    for (inner, row) in child.children.iter().enumerate() {
                        let (row_style, _) = Style::read(&row.props);
                        if parts.set_aside(
                            &mut current.children[index].children[inner],
                            row,
                            &row_style,
                            at,
                        ) {
                            continue;
                        }
                        parts.add_row(current, node, at.then(inner), row_style, Some(group));
                    }
                    parts.groups[group].rows.1 = parts.rows.len();
                }
                TablePart::Caption if own == TablePart::Table => parts.captions.push((at, style)),
                TablePart::Column | TablePart::Columns => parts.add_columns(child, &style),
                _ => parts.add_row(current, node, at, style, None),
            }
        }
        parts
    }

    /// Puts each cell in its column, after what's already there, as a cell in a row above that goes down into this one is.
    /// Gives back how many columns there are.
    fn place(&mut self) -> usize {
        let rows = self.rows.len();
        let mut taken: Vec<Vec<bool>> = vec![Vec::new(); rows];
        let mut columns = self.said_widths.len();
        let mut next = vec![0usize; rows];
        for cell in &mut self.cells {
            let row = cell.row;
            let mut column = next[row];
            while taken[row].get(column).copied().unwrap_or(false) {
                column += 1;
            }
            // Down as far as it says, or to the end with none, but no further than there are rows.
            let down = if cell.down == 0 {
                rows - row
            } else {
                cell.down.min(rows - row)
            };
            cell.column = column;
            cell.down = down.max(1);
            for line in &mut taken[row..row + cell.down] {
                if line.len() < column + cell.across {
                    line.resize(column + cell.across, false);
                }
                line[column..column + cell.across].fill(true);
            }
            next[row] = column + cell.across;
            columns = columns.max(column + cell.across);
        }
        columns
    }
}

/// Shares `room` out between `sizes`, after what each needs at least, by how much each would like more than that. What's left
/// once each has all it would like goes to the columns whose width isn't said (`said`), as CSS's automatic table layout has it:
/// a `w-[100px]` column stays 100px wide while the others take the rest. Only if every width is said do they all share it.
fn widen(sizes: &mut [f32], least: &[f32], most: &[f32], said: &[bool], room: f32) {
    let least_total: f32 = least.iter().sum();
    let most_total: f32 = most.iter().sum();
    if room >= most_total {
        // As wide as they'd like, and what's left by how wide that is, or the same for each if none would like any.
        let spare = room - most_total;
        let free = |at: usize| !said.get(at).copied().unwrap_or(false);
        let any_free = (0..sizes.len()).any(free);
        let takes = |at: usize| !any_free || free(at);
        let count = (0..sizes.len()).filter(|at| takes(*at)).count().max(1) as f32;
        let by_total: f32 = (0..sizes.len()).filter(|at| takes(*at)).map(|at| most[at]).sum();
        for (at, size) in sizes.iter_mut().enumerate() {
            *size = most[at]
                + if !takes(at) {
                    0.0
                } else if by_total > 0.0 {
                    spare * most[at] / by_total
                } else {
                    spare / count
                };
        }
    } else if room > least_total && most_total > least_total {
        let share = (room - least_total) / (most_total - least_total);
        for (at, size) in sizes.iter_mut().enumerate() {
            *size = least[at] + (most[at] - least[at]) * share;
        }
    } else {
        sizes.copy_from_slice(least);
    }
}

/// What's left of `wanted` once the columns from `start` to `start + across`, and the gaps between them, have been taken from it,
/// shared out among those columns by `by`, or the same for each if that's nothing.
fn spread_over(sizes: &mut [f32], by: &[f32], start: usize, across: usize, wanted: f32, gap: f32) {
    let end = (start + across).min(sizes.len());
    let have = sizes[start..end].iter().sum::<f32>() + gap * end.saturating_sub(start + 1) as f32;
    if wanted <= have || end <= start {
        return;
    }
    let more = wanted - have;
    let total: f32 = by[start..end].iter().sum();
    let count = (end - start) as f32;
    for at in start..end {
        sizes[at] += if total > 0.0 {
            more * by[at] / total
        } else {
            more / count
        };
    }
}

impl<M: Measure> Pass<'_, M> {
    #[inline(never)]
    pub(super) fn solve_table(
        &mut self,
        current: &mut LayoutBox,
        node: &Element,
        style: &Style,
        key: Key,
        mode: Mode,
    ) -> Size {
        let names = &*NAMES;
        let (pad_x, pad_y) = padding_totals(style);
        let gap_of = |name| match node.props.get_symbol(name) {
            Some(PropValue::Float(gap)) => *gap as f32,
            Some(PropValue::Int(gap)) => *gap as f32,
            _ => style.gap,
        };
        let (mut column_gap, mut row_gap) = (gap_of(names.column_gap), gap_of(names.row_gap));
        let mut parts = Parts::find(current, node, style.table);
        let collapse = matches!(
            node.props.get_symbol(names.table_collapse),
            Some(PropValue::Bool(true))
        );
        if collapse {
            // Borders next to each other are one, as `border-collapse: collapse` has them, which is as much as one of them over
            // the other. Where cells only have a border on one side, as a line under each row is, there's nothing for it to be
            // one with.
            let mut most = [0f32; 4];
            for cell in &parts.cells {
                for (side, width) in super::border_sides(node_at(node, cell.at))
                    .into_iter()
                    .enumerate()
                {
                    most[side] = most[side].max(width);
                }
            }
            let [top, right, bottom, left] = most;
            column_gap = -right.min(left);
            row_gap = -top.min(bottom);
        }
        let columns = parts.place();
        let gaps = |count: usize, gap: f32| gap * count.saturating_sub(1) as f32;

        // How wide each column could be, at least and at most, from the cells in it, the ones in one column first, and then
        // those across more than one, which widen what they're across if that isn't enough for them.
        let fixed = matches!(
            node.props.get_symbol(names.table_fixed),
            Some(PropValue::Bool(true))
        );
        let mut least = vec![0f32; columns];
        let mut most = vec![0f32; columns];
        // Which columns say how wide they are, by a `<col>` or by a cell of their own.
        let mut said_width = vec![false; columns];
        for (at, said) in parts.said_widths.iter().enumerate() {
            if let Some(width) = said {
                least[at] = *width;
                most[at] = *width;
                if let Some(one) = said_width.get_mut(at) {
                    *one = true;
                }
            }
        }
        if fixed {
            // With `table-layout: fixed`, it's what's said for each column, or by the cells of the first row, and the rest share
            // what's left, as nothing in a cell is asked anything.
            for cell in parts
                .cells
                .iter()
                .filter(|cell| cell.row == 0 && cell.across == 1)
            {
                if parts
                    .said_widths
                    .get(cell.column)
                    .copied()
                    .flatten()
                    .is_none()
                {
                    if let Some(width) = resolve_length(cell.style.width, None) {
                        least[cell.column] = width;
                        most[cell.column] = width;
                    }
                }
            }
        } else {
            let mut spanning = Vec::new();
            for (at, cell) in parts.cells.iter().enumerate() {
                let cell_box = box_at(current, cell.at);
                let cell_node = node_at(node, cell.at);
                let ask = |otherwise| Key {
                    width: asked_about(
                        cell.style.width,
                        otherwise,
                        cell.style.min_width,
                        cell.style.max_width,
                    ),
                    height: Axis::UNBOUNDED,
                };
                let narrowest = self
                    .solve_styled(
                        cell_box,
                        cell_node,
                        &cell.style,
                        ask(Axis::MIN_CONTENT),
                        Mode::Measure,
                    )
                    .width
                    + cell.margin_x();
                let widest = self
                    .solve_styled(
                        cell_box,
                        cell_node,
                        &cell.style,
                        ask(Axis::UNBOUNDED),
                        Mode::Measure,
                    )
                    .width
                    + cell.margin_x();
                let widest = widest.max(narrowest);
                if cell.across == 1 {
                    if resolve_length(cell.style.width, None).is_some() {
                        said_width[cell.column] = true;
                    }
                    least[cell.column] = least[cell.column].max(narrowest);
                    most[cell.column] = most[cell.column].max(widest);
                } else {
                    spanning.push((at, narrowest, widest));
                }
            }
            spanning.sort_by_key(|&(at, ..)| parts.cells[at].across);
            for (at, narrowest, widest) in spanning {
                let cell = &parts.cells[at];
                let before = most.clone();
                spread_over(
                    &mut least,
                    &before,
                    cell.column,
                    cell.across,
                    narrowest,
                    column_gap,
                );
                spread_over(
                    &mut most,
                    &before,
                    cell.column,
                    cell.across,
                    widest,
                    column_gap,
                );
            }
        }
        for at in 0..columns {
            most[at] = most[at].max(least[at]);
        }

        // How wide it is inside: as it's been told, or as wide as its columns would like, as far as there's room, but never
        // narrower than they need.
        let all_gaps = gaps(columns, column_gap);
        let least_total = least.iter().sum::<f32>() + all_gaps;
        let most_total = most.iter().sum::<f32>() + all_gaps;
        let inside = if key.width.exact {
            (key.width.value - pad_x).max(0.0)
        } else if key.width.value < 0.0 {
            least_total
        } else {
            most_total
                .min((key.width.value - pad_x).max(0.0))
                .max(least_total)
        };
        // As wide as its `min-width` says, with the columns sharing what that adds, as `min-width: 100%` makes Radix ScrollArea's
        // `display: table` content as wide as its viewport. A share is of the room it's given.
        let inside = if key.width.exact {
            inside
        } else {
            let mut least_wide = style.min_width;
            if let Some(share) = style.min_width_percent
                && key.width.value.is_finite()
                && key.width.value >= 0.0
            {
                least_wide = least_wide.max(share.at(key.width.value));
            }
            inside.max(least_wide - pad_x)
        };
        let mut widths = vec![0f32; columns];
        if fixed {
            // What says nothing shares what's left, the same for each.
            let said: f32 = least.iter().sum();
            let unsaid = least.iter().filter(|width| **width == 0.0).count();
            let spare = (inside - all_gaps - said).max(0.0);
            for at in 0..columns {
                widths[at] = if least[at] == 0.0 && unsaid > 0 {
                    spare / unsaid as f32
                } else {
                    least[at]
                };
            }
            if unsaid == 0 {
                widen(&mut widths, &least, &least, &[], inside - all_gaps);
            }
        } else {
            widen(&mut widths, &least, &most, &said_width, (inside - all_gaps).max(0.0));
        }
        let width_of = |column: usize, across: usize| {
            let end = (column + across).min(columns);
            widths[column..end].iter().sum::<f32>() + gaps(end.saturating_sub(column), column_gap)
        };
        let content_width = widths.iter().sum::<f32>() + all_gaps;

        // How tall each row is, now that the widths are known, the cells in one row first, and then those down more than one,
        // which make the last of their rows taller if they need it.
        let mut heights: Vec<f32> = parts.rows.iter().map(|row| row.height).collect();
        // A row's own borders take room in its cells, as `border-collapse: collapse` draws them, which shadcn's `border-b` on each
        // `<tr>` is: a 36px row of cells is 37px with its line under it, while a cell that says how tall it is (box-sizing:
        // border-box, as Tailwind's preflight has everything) has the line inside that, as `h-10` heads are 40px.
        // A group's borders, as a `<tfoot>`'s `border-t`, take room in its first and last rows the same way. Borders that meet are
        // one, as wide as the widest, so what the row's cells have on that side already takes all the room it needs.
        let mut row_sides: Vec<[f32; 2]> = parts
            .rows
            .iter()
            .map(|row| match row.at {
                Some(path) if collapse => {
                    let [top, _, bottom, _] = super::border_sides(node_at(node, path));
                    [top, bottom]
                }
                _ => [0.0; 2],
            })
            .collect();
        if collapse {
            for group in &parts.groups {
                let (first, end) = group.rows;
                if end > first {
                    let [top, _, bottom, _] = super::border_sides(node_at(node, group.at));
                    row_sides[first][0] = row_sides[first][0].max(top);
                    row_sides[end - 1][1] = row_sides[end - 1][1].max(bottom);
                }
            }
        }
        let mut cell_sides = vec![[0f32; 2]; parts.rows.len()];
        if collapse {
            for cell in &parts.cells {
                let [top, _, bottom, _] = super::border_sides(node_at(node, cell.at));
                let sides = &mut cell_sides[cell.row];
                sides[0] = sides[0].max(top);
                sides[1] = sides[1].max(bottom);
            }
        }
        let row_borders: Vec<f32> = row_sides
            .iter()
            .zip(&cell_sides)
            .map(|(row, cells)| (row[0] - cells[0]).max(0.0) + (row[1] - cells[1]).max(0.0))
            .collect();
        let mut tall = Vec::new();
        for (at, cell) in parts.cells.iter().enumerate() {
            let cell_box = box_at(current, cell.at);
            let cell_node = node_at(node, cell.at);
            let width = (width_of(cell.column, cell.across) - cell.margin_x()).max(0.0);
            let height = self
                .solve_styled(
                    cell_box,
                    cell_node,
                    &cell.style,
                    Key {
                        width: Axis::exact(width),
                        height: asked_about(
                            cell.style.height,
                            Axis::UNBOUNDED,
                            cell.style.min_height,
                            cell.style.max_height,
                        ),
                    },
                    Mode::Measure,
                )
                .height
                + cell.margin_y();
            let height = if resolve_length(cell.style.height, None).is_none() {
                height + row_borders[cell.row]
            } else {
                height
            };
            if cell.down == 1 {
                heights[cell.row] = heights[cell.row].max(height);
            } else {
                tall.push((at, height));
            }
        }
        for (at, height) in tall {
            let cell = &parts.cells[at];
            let end = cell.row + cell.down;
            let have = heights[cell.row..end].iter().sum::<f32>() + gaps(cell.down, row_gap);
            if height > have {
                heights[end - 1] += height - have;
            }
        }
        let mut captions_height = 0.0;
        let mut top_captions_height = 0.0;
        let mut caption_heights = Vec::with_capacity(parts.captions.len());
        for (at, caption_style) in &parts.captions {
            let caption = node_at(node, *at);
            let [margin_top, _, margin_bottom, _] = if caption_style.has_margin {
                Style::read_margins(&caption.props)
            } else {
                [0.0; 4]
            };
            let height = self
                .solve_styled(
                    box_at(current, *at),
                    node_at(node, *at),
                    caption_style,
                    Key {
                        width: Axis::exact(content_width),
                        height: Axis::UNBOUNDED,
                    },
                    Mode::Measure,
                )
                .height;
            caption_heights.push((height, margin_top));
            captions_height += height + margin_top + margin_bottom;
            if !below_rows(node, caption) {
                top_captions_height += height + margin_top + margin_bottom;
            }
        }
        let rows_height = heights.iter().sum::<f32>() + gaps(heights.len(), row_gap);
        // Told to be taller than its rows, they share what's left.
        if key.height.exact && !heights.is_empty() {
            let room = key.height.value - pad_y - captions_height;
            if room > rows_height {
                let more = (room - rows_height) / heights.len() as f32;
                for height in &mut heights {
                    *height += more;
                }
            }
        }
        let size = Size::new(
            resolve_auto(
                key.width,
                content_width + pad_x,
                style.min_width,
                style.max_width,
            ),
            resolve_auto(
                key.height,
                captions_height + rows_height + pad_y,
                style.min_height,
                style.max_height,
            ),
        );
        // Rows and groups are never asked anything, so they're never dirty once the table's been.
        let mut tidy = |at: Path| {
            let one = box_at(current, at);
            one.flags &= !(SELF_DIRTY | CHILD_DIRTY | MANY_CHILDREN_DIRTY);
            one.clear_results();
        };
        for group in &parts.groups {
            tidy(group.at);
        }
        for row in &parts.rows {
            if let Some(at) = row.at {
                tidy(at);
            }
        }
        if mode == Mode::Measure {
            return size;
        }

        // Place everything.
        let mut top = style.padding_top;
        // What's said about the table goes above its rows, or below them, as its `caption-side` says.
        let mut below = style.padding_top + top_captions_height + rows_height;
        for ((at, caption_style), (height, margin_top)) in parts.captions.iter().zip(caption_heights) {
            let caption = node_at(node, *at);
            let [_, _, margin_bottom, _] = if caption_style.has_margin {
                Style::read_margins(&caption.props)
            } else {
                [0.0; 4]
            };
            let place = if below_rows(node, caption) { &mut below } else { &mut top };
            let caption_box = box_at(current, *at);
            self.solve_styled(
                caption_box,
                node_at(node, *at),
                caption_style,
                Key {
                    width: Axis::exact(content_width),
                    height: Axis::exact(height),
                },
                Mode::Layout,
            );
            caption_box.x = style.padding_left;
            caption_box.y = *place + margin_top;
            *place += height + margin_top + margin_bottom;
        }
        let mut row_tops = Vec::with_capacity(heights.len());
        for height in &heights {
            row_tops.push(top);
            top += height + row_gap;
        }
        let mut column_starts = Vec::with_capacity(columns);
        let mut left = style.padding_left;
        for width in &widths {
            column_starts.push(left);
            left += width + column_gap;
        }
        // Where text goes from right to left, the first column is at the right, as a grid's is (`solve_grid`): each cell is as
        // far from the right of the columns as it would be from their left.
        let columns_width = if widths.is_empty() { 0.0 } else { left - column_gap - style.padding_left };
        let mirrored = crate::engine::right_to_left(node);
        let rows_end =
            |first: usize, end: usize| row_tops[end - 1] + heights[end - 1] - row_tops[first];
        let mut group_places = Vec::with_capacity(parts.groups.len());
        for group in &parts.groups {
            let (first, end) = group.rows;
            let y = row_tops.get(first).copied().unwrap_or(top);
            let height = if end > first {
                rows_end(first, end)
            } else {
                0.0
            };
            let group_box = box_at(current, group.at);
            group_box.x = style.padding_left;
            group_box.y = y;
            group_box.width = content_width;
            group_box.height = height;
            group_places.push((style.padding_left, y));
        }
        // Where what each row's in is from the table's top left, as what's in it is placed from there.
        let parent_of = |row: &Row| row.group.map_or((0.0, 0.0), |group| group_places[group]);
        for (at, row) in parts.rows.iter().enumerate() {
            if let Some(path) = row.at {
                let (x, y) = parent_of(row);
                let row_box = box_at(current, path);
                row_box.x = style.padding_left - x;
                row_box.y = row_tops[at] - y;
                row_box.width = content_width;
                row_box.height = heights[at];
            }
        }
        for cell in &parts.cells {
            let row = &parts.rows[cell.row];
            let (x, y) = if row.at.is_some() {
                (style.padding_left, row_tops[cell.row])
            } else {
                parent_of(row)
            };
            let width = (width_of(cell.column, cell.across) - cell.margin_x()).max(0.0);
            let end = cell.row + cell.down;
            let height = rows_end(cell.row, end) - cell.margin_y();
            let cell_box = box_at(current, cell.at);
            self.solve_styled(
                cell_box,
                node_at(node, cell.at),
                &cell.style,
                Key {
                    width: Axis::exact(width),
                    height: Axis::exact(height.clamp_to(0.0, f32::INFINITY)),
                },
                Mode::Layout,
            );
            let start = if mirrored {
                2.0 * style.padding_left + columns_width - column_starts[cell.column] - width_of(cell.column, cell.across)
            } else {
                column_starts[cell.column]
            };
            cell_box.x = start - x + cell.margins[3];

            cell_box.y = row_tops[cell.row] - y + cell.margins[0];
        }
        for at in parts.overlays.iter().rev() {
            self.solve_overlays(box_at(current, *at), node_at(node, *at));
        }
        for at in parts.absolutes.iter().rev() {
            // The table's own size is only kept once this has finished.
            let outer = if at.len == 0 {
                size
            } else {
                let one = box_at(current, *at);
                Size::new(one.width, one.height)
            };
            self.solve_absolutes(box_at(current, *at), node_at(node, *at), outer);
        }
        // What's on top of everything is found by going down only where there's something, so each row and group says.
        for row in &parts.rows {
            if let Some(at) = row.at {
                box_at(current, at).update_contains_overlay();
                box_at(current, at).update_contains_absolute();
            }
        }
        for group in &parts.groups {
            box_at(current, group.at).update_contains_overlay();
            box_at(current, group.at).update_contains_absolute();
        }
        current.update_contains_overlay();
        size
    }
}
