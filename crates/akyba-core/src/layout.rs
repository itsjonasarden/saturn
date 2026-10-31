//! The terminal grid as data: rows of cells, each cell holding tabs. Operations change the layout
//! only, never the terminals, so the grid can be rearranged freely while terminals keep running.

/// A terminal tab. Akyba uses the terminal's id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowId(u64);

impl CellId {
    /// A number that stays the same for the cell's whole life, e.g. for UI element ids.
    pub fn key(self) -> u64 {
        self.0
    }
}

impl RowId {
    /// A number that stays the same for the row's whole life, e.g. for UI element ids.
    pub fn key(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub id: CellId,
    pub tabs: Vec<TabId>,
    pub active: TabId,
    /// Relative width inside its row.
    pub size: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: RowId,
    pub cells: Vec<Cell>,
    /// Relative height inside the grid.
    pub size: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Where a cell sits: row index and position in the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Position {
    row: usize,
    cell: usize,
}

/// The grid's shape without its terminals, to save and restore: row and cell sizes, how many tabs
/// each cell holds, which is in front, and which cell has focus.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Shape {
    pub rows: Vec<RowShape>,
    /// (row, cell) of the focused cell.
    pub focused: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RowShape {
    pub size: f32,
    pub cells: Vec<CellShape>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CellShape {
    pub size: f32,
    pub tabs: usize,
    /// Index of the tab in front.
    pub active: usize,
}

impl Shape {
    /// How many tabs the grid holds, in the order `Layout::tabs` lists them.
    pub fn tab_count(&self) -> usize {
        self.rows.iter().flat_map(|row| &row.cells).map(|cell| cell.tabs).sum()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub rows: Vec<Row>,
    pub focused: Option<CellId>,
    pub maximized: Option<CellId>,
    next_id: u64,
}

impl Layout {
    /// The grid's shape, for saving. Tabs are listed by `tabs()` in the same order.
    pub fn shape(&self) -> Shape {
        let focused = self.focused.and_then(|id| self.position(id)).map(|at| (at.row, at.cell));
        let rows = self
            .rows
            .iter()
            .map(|row| RowShape {
                size: row.size,
                cells: row
                    .cells
                    .iter()
                    .map(|cell| CellShape {
                        size: cell.size,
                        tabs: cell.tabs.len(),
                        active: cell.tabs.iter().position(|tab| *tab == cell.active).unwrap_or(0),
                    })
                    .collect(),
            })
            .collect();
        Shape { rows, focused }
    }

    /// A grid of `shape`, filled with `tabs` in order. Cells left without tabs (too few given)
    /// are dropped, and so are rows left without cells. A cell saved with several tabs, from
    /// before each pane held one terminal, becomes as many panes side by side in its place.
    pub fn from_shape(shape: &Shape, tabs: &[TabId]) -> Self {
        let mut layout = Self::default();
        let mut next = tabs.iter().copied();
        let mut focused = None;
        for (row_index, row_shape) in shape.rows.iter().enumerate() {
            let mut cells = Vec::new();
            for (cell_index, cell_shape) in row_shape.cells.iter().enumerate() {
                let cell_tabs: Vec<TabId> = next.by_ref().take(cell_shape.tabs).collect();
                #[expect(clippy::cast_precision_loss, reason = "a cell held a handful of tabs")]
                let each = cell_shape.size / cell_tabs.len().max(1) as f32;
                for (index, tab) in cell_tabs.into_iter().enumerate() {
                    let cell = layout.new_cell(tab, each);
                    if shape.focused == Some((row_index, cell_index)) && index == cell_shape.active {
                        focused = Some(cell.id);
                    }
                    cells.push(cell);
                }
            }
            if !cells.is_empty() {
                let id = RowId(layout.new_id());
                layout.rows.push(Row { id, cells, size: row_shape.size });
            }
        }
        layout.focused = focused.or_else(|| layout.cells().next().map(|cell| cell.id));
        // Cells and rows dropped above leave their share behind; so could a grid saved before
        // closing panes gave their space back.
        layout.fill();
        layout
    }

    /// Scales the sizes so the rows fill the grid and each row's cells fill the row: what a
    /// closed cell or row took goes to the ones left, in proportion.
    fn fill(&mut self) {
        let total: f32 = self.rows.iter().map(|row| row.size).sum();
        for row in &mut self.rows {
            row.size = if total > 0.0 { row.size / total } else { 1.0 };
            let total: f32 = row.cells.iter().map(|cell| cell.size).sum();
            for cell in &mut row.cells {
                cell.size = if total > 0.0 { cell.size / total } else { 1.0 };
            }
        }
    }

    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn new_cell(&mut self, tab: TabId, size: f32) -> Cell {
        Cell { id: CellId(self.new_id()), tabs: vec![tab], active: tab, size }
    }

    /// Every cell, row by row, left to right: the order ⌘1…⌘9 count in.
    pub fn cells(&self) -> impl Iterator<Item = &Cell> {
        self.rows.iter().flat_map(|row| row.cells.iter())
    }

    pub fn tabs(&self) -> impl Iterator<Item = TabId> + '_ {
        self.cells().flat_map(|cell| cell.tabs.iter().copied())
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    fn position(&self, cell: CellId) -> Option<Position> {
        self.rows.iter().enumerate().find_map(|(row_index, row)| {
            let cell_index = row.cells.iter().position(|c| c.id == cell)?;
            Some(Position { row: row_index, cell: cell_index })
        })
    }

    fn position_of_tab(&self, tab: TabId) -> Option<Position> {
        self.rows.iter().enumerate().find_map(|(row_index, row)| {
            let cell_index = row.cells.iter().position(|c| c.tabs.contains(&tab))?;
            Some(Position { row: row_index, cell: cell_index })
        })
    }

    fn cell_at(&mut self, at: Position) -> &mut Cell {
        &mut self.rows[at.row].cells[at.cell]
    }

    pub fn cell(&self, id: CellId) -> Option<&Cell> {
        self.cells().find(|cell| cell.id == id)
    }

    pub fn cell_of_tab(&self, tab: TabId) -> Option<&Cell> {
        self.cells().find(|cell| cell.tabs.contains(&tab))
    }

    /// The focused cell, or the first one when focus was lost.
    fn target(&self, cell: Option<CellId>) -> Option<Position> {
        cell.or(self.focused)
            .and_then(|id| self.position(id))
            .or_else(|| (!self.rows.is_empty()).then_some(Position { row: 0, cell: 0 }))
    }

    /// The tab the user is looking at: the focused cell's active tab.
    pub fn focused_tab(&self) -> Option<TabId> {
        self.target(None).map(|at| self.rows[at.row].cells[at.cell].active)
    }

    /// Tabs drawn on screen right now: each visible cell's active tab.
    pub fn visible_tabs(&self) -> Vec<TabId> {
        self.cells().filter(|cell| self.maximized.is_none_or(|max| max == cell.id)).map(|cell| cell.active).collect()
    }

    /// Adds a pane for a tab: the grid's first, or to the right of a cell (the focused one by
    /// default). Every pane holds one terminal.
    pub fn add_tab(&mut self, tab: TabId, cell: Option<CellId>) {
        if self.target(cell).is_some() {
            return self.split_right(tab, cell);
        }
        let cell = self.new_cell(tab, 1.0);
        let id = RowId(self.new_id());
        self.focused = Some(cell.id);
        self.maximized = None;
        self.rows = vec![Row { id, cells: vec![cell], size: 1.0 }];
    }

    /// Opens a tab in a new cell to the right of a cell; the row's panes share its width evenly.
    pub fn split_right(&mut self, tab: TabId, cell: Option<CellId>) {
        let Some(at) = self.target(cell) else { return self.add_tab(tab, None) };
        let new_cell = self.new_cell(tab, 1.0);
        self.focused = Some(new_cell.id);
        self.maximized = None;
        let row = &mut self.rows[at.row];
        row.cells.insert(at.cell + 1, new_cell);
        even_out(row.cells.iter_mut().map(|cell| &mut cell.size));
    }

    /// Opens a tab in a new row below a cell's row; the rows share the height evenly.
    pub fn split_down(&mut self, tab: TabId, cell: Option<CellId>) {
        let Some(at) = self.target(cell) else { return self.add_tab(tab, None) };
        let new_cell = self.new_cell(tab, 1.0);
        let id = RowId(self.new_id());
        self.focused = Some(new_cell.id);
        self.maximized = None;
        self.rows.insert(at.row + 1, Row { id, cells: vec![new_cell], size: 1.0 });
        even_out(self.rows.iter_mut().map(|row| &mut row.size));
    }

    /// Moves a tab out of its cell into a new cell beside `target`: left or right of it in its row,
    /// or in a new row above or below. Its old cell goes when it empties. Nothing happens when the
    /// tab is the only one in `target` itself: a cell can't be split off from itself.
    pub fn split_with(&mut self, tab: TabId, target: CellId, side: Direction) {
        let alone_in_target = self.cell(target).is_some_and(|cell| cell.tabs == [tab]);
        if alone_in_target || self.position_of_tab(tab).is_none() || self.position(target).is_none() {
            return;
        }
        self.remove_tab(tab);
        let Some(at) = self.position(target) else { return };
        self.maximized = None;
        match side {
            Direction::Left | Direction::Right => {
                let new_cell = self.new_cell(tab, 1.0);
                self.focused = Some(new_cell.id);
                let row = &mut self.rows[at.row];
                let index = if side == Direction::Right { at.cell + 1 } else { at.cell };
                row.cells.insert(index, new_cell);
                even_out(row.cells.iter_mut().map(|cell| &mut cell.size));
            }
            Direction::Up | Direction::Down => {
                let new_cell = self.new_cell(tab, 1.0);
                self.focused = Some(new_cell.id);
                let id = RowId(self.new_id());
                let index = if side == Direction::Down { at.row + 1 } else { at.row };
                self.rows.insert(index, Row { id, cells: vec![new_cell], size: 1.0 });
                even_out(self.rows.iter_mut().map(|row| &mut row.size));
            }
        }
        self.fill();
    }

    /// Takes a tab out of the grid; an emptied cell or row goes with it, and the rest share its
    /// space evenly.
    pub fn remove_tab(&mut self, tab: TabId) {
        let Some(at) = self.position_of_tab(tab) else { return };
        let cell = self.cell_at(at);
        let index = cell.tabs.iter().position(|t| *t == tab).unwrap_or(0);
        cell.tabs.remove(index);
        if !cell.tabs.is_empty() {
            if cell.active == tab {
                cell.active = cell.tabs[index.saturating_sub(1).min(cell.tabs.len() - 1)];
            }
            return;
        }

        let removed = cell.id;
        let order: Vec<CellId> = self.cells().map(|c| c.id).collect();
        let place = order.iter().position(|id| *id == removed).unwrap_or(0);
        let neighbour = place.checked_sub(1).and_then(|i| order.get(i)).or_else(|| order.get(place + 1)).copied();
        self.rows[at.row].cells.remove(at.cell);
        if self.rows[at.row].cells.is_empty() {
            self.rows.remove(at.row);
            even_out(self.rows.iter_mut().map(|row| &mut row.size));
        } else {
            even_out(self.rows[at.row].cells.iter_mut().map(|cell| &mut cell.size));
        }
        self.fill();
        if self.focused == Some(removed) {
            self.focused = neighbour;
        }
        if self.maximized == Some(removed) {
            self.maximized = None;
        }
    }

    /// Moves a tab into a cell (or to another position in its own), making it active there.
    /// Trades the places of a tab's pane and `to`: what a pane dropped on another's middle does.
    pub fn swap(&mut self, tab: TabId, to: CellId) {
        let (Some(from), Some(at)) = (self.position_of_tab(tab), self.position(to)) else { return };
        if from == at {
            return;
        }
        let other = self.cell_at(at).active;
        for (position, holds) in [(from, other), (at, tab)] {
            let cell = self.cell_at(position);
            cell.tabs = vec![holds];
            cell.active = holds;
        }
        self.focused = Some(to);
    }

    pub fn activate_tab(&mut self, tab: TabId) {
        if let Some(at) = self.position_of_tab(tab) {
            let cell = self.cell_at(at);
            cell.active = tab;
            self.focused = Some(cell.id);
        }
    }

    pub fn focus_cell(&mut self, cell: CellId) {
        if self.position(cell).is_some() {
            self.focused = Some(cell);
        }
    }

    /// ⌘1…⌘9: the nth cell in reading order (1-based).
    pub fn focus_nth(&mut self, n: usize) {
        let Some(id) = n.checked_sub(1).and_then(|i| self.cells().nth(i)).map(|c| c.id) else { return };
        self.focused = Some(id);
        if self.maximized.is_some() {
            self.maximized = Some(id);
        }
    }

    /// The next or previous pane in reading order, wrapping around.
    pub fn cycle_pane(&mut self, forward: bool) {
        let order: Vec<CellId> = self.cells().map(|cell| cell.id).collect();
        let len = order.len();
        if len == 0 {
            return;
        }
        let index = self.focused.and_then(|focused| order.iter().position(|id| *id == focused)).unwrap_or(0);
        let next = order[if forward { (index + 1) % len } else { (index + len - 1) % len }];
        self.focused = Some(next);
        if self.maximized.is_some() {
            self.maximized = Some(next);
        }
    }

    /// Moves focus to the neighbouring cell; up and down pick the cell most directly above or below.
    pub fn focus_direction(&mut self, direction: Direction) {
        let Some(from) = self.target(None) else { return };
        let target = match direction {
            Direction::Left => from.cell.checked_sub(1).map(|cell| Position { cell, ..from }),
            Direction::Right => {
                (from.cell + 1 < self.rows[from.row].cells.len()).then_some(Position { cell: from.cell + 1, ..from })
            }
            Direction::Up | Direction::Down => {
                let row = if direction == Direction::Up { from.row.checked_sub(1) } else { Some(from.row + 1) };
                row.filter(|r| *r < self.rows.len()).map(|row| {
                    let x = centres(&self.rows[from.row])[from.cell];
                    let candidates = centres(&self.rows[row]);
                    let mut best = 0;
                    for (index, centre) in candidates.iter().enumerate() {
                        if (centre - x).abs() < (candidates[best] - x).abs() {
                            best = index;
                        }
                    }
                    Position { row, cell: best }
                })
            }
        };
        if let Some(at) = target {
            self.focused = Some(self.rows[at.row].cells[at.cell].id);
        }
    }

    pub fn toggle_maximize(&mut self) {
        self.maximized = if self.maximized.is_some() { None } else { self.focused };
    }

    /// Every row the same height and every pane in a row the same width, undoing dragged dividers.
    pub fn even_out(&mut self) {
        even_out(self.rows.iter_mut().map(|row| &mut row.size));
        for row in &mut self.rows {
            even_out(row.cells.iter_mut().map(|cell| &mut cell.size));
        }
    }

    pub fn resize_rows(&mut self, sizes: &[f32]) {
        for (row, size) in self.rows.iter_mut().zip(sizes) {
            row.size = *size;
        }
    }

    pub fn resize_cells(&mut self, row: RowId, sizes: &[f32]) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.id == row) {
            for (cell, size) in row.cells.iter_mut().zip(sizes) {
                cell.size = *size;
            }
        }
    }
}

/// The horizontal centre of each cell in a row, from 0 to 1.
/// Every pane in a row, or every row, the same size: what a grid gets whenever a pane comes or
/// goes, or Even out panes is asked for. Sizes dragged by hand last until then.
fn even_out<'a>(sizes: impl Iterator<Item = &'a mut f32>) {
    let mut sizes: Vec<&mut f32> = sizes.collect();
    #[expect(clippy::cast_precision_loss, reason = "a row holds a handful of panes")]
    let count = sizes.len() as f32;
    for size in &mut sizes {
        **size = 1.0 / count;
    }
}

fn centres(row: &Row) -> Vec<f32> {
    let total: f32 = row.cells.iter().map(|c| c.size).sum();
    let mut start = 0.0;
    row.cells
        .iter()
        .map(|cell| {
            let centre = (start + cell.size / 2.0) / total;
            start += cell.size;
            centre
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: TabId = TabId(1);
    const B: TabId = TabId(2);
    const C: TabId = TabId(3);
    const D: TabId = TabId(4);
    const E: TabId = TabId(5);

    /// The grid as tab numbers per cell, row by row.
    fn shape(layout: &Layout) -> Vec<Vec<Vec<u64>>> {
        layout
            .rows
            .iter()
            .map(|row| row.cells.iter().map(|cell| cell.tabs.iter().map(|t| t.0).collect()).collect())
            .collect()
    }

    fn cell_of(layout: &Layout, tab: TabId) -> CellId {
        layout.cell_of_tab(tab).map(|c| c.id).expect("tab is in the grid")
    }

    #[test]
    fn starts_a_grid_with_the_first_tab() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        assert_eq!(shape(&layout), [[[1]]]);
        assert_eq!(layout.focused_tab(), Some(A));
    }

    #[test]
    fn a_new_tab_is_a_pane_of_its_own() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.add_tab(B, None);
        layout.focus_nth(1);
        layout.add_tab(C, None);
        assert_eq!(shape(&layout), [[[1], [3], [2]]]);
        assert_eq!(layout.focused_tab(), Some(C));
    }

    #[test]
    fn a_cell_saved_with_several_tabs_opens_as_that_many_panes() {
        let saved = Shape {
            rows: vec![RowShape {
                size: 1.0,
                cells: vec![CellShape { size: 0.4, tabs: 1, active: 0 }, CellShape { size: 0.6, tabs: 3, active: 1 }],
            }],
            focused: Some((0, 1)),
        };
        let layout = Layout::from_shape(&saved, &[A, B, C, D]);
        assert_eq!(shape(&layout), [[[1], [2], [3], [4]]]);
        assert_eq!(layout.focused_tab(), Some(C));
        let sizes: Vec<f32> = layout.rows[0].cells.iter().map(|c| c.size).collect();
        assert!((sizes[1] - 0.2).abs() < 1e-6, "{sizes:?}");
    }

    #[test]
    fn splits_right_into_a_cell_sharing_the_width() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.split_right(B, None);
        assert_eq!(shape(&layout), [[[1], [2]]]);
        assert_eq!(layout.rows[0].cells.iter().map(|c| c.size).collect::<Vec<_>>(), [0.5, 0.5]);
        assert_eq!(layout.focused_tab(), Some(B));
    }

    #[test]
    fn adding_panes_evens_the_row_out() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        for tab in [B, C, D, E] {
            layout.split_right(tab, None);
        }
        let sizes: Vec<f32> = layout.rows[0].cells.iter().map(|c| c.size).collect();
        assert!(sizes.iter().all(|size| (size - 0.2).abs() < 1e-6), "{sizes:?}");
    }

    #[test]
    fn evens_out_every_row_and_pane_after_dragging() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.split_right(B, None);
        layout.split_down(C, None);
        layout.resize_rows(&[0.8, 0.2]);
        let row = layout.rows[0].id;
        layout.resize_cells(row, &[0.9, 0.1]);
        layout.even_out();
        let rows: Vec<f32> = layout.rows.iter().map(|r| r.size).collect();
        let cells: Vec<f32> = layout.rows[0].cells.iter().map(|c| c.size).collect();
        assert_eq!((rows, cells), (vec![0.5, 0.5], vec![0.5, 0.5]));
    }

    #[test]
    fn splits_down_into_a_new_row() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.split_right(B, None);
        layout.split_down(C, None);
        assert_eq!(shape(&layout), vec![vec![vec![1], vec![2]], vec![vec![3]]]);
        assert_eq!(layout.rows.iter().map(|r| r.size).collect::<Vec<_>>(), [0.5, 0.5]);
    }

    #[test]
    fn closing_a_pane_gives_its_space_to_the_rest() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.split_down(B, None);
        layout.resize_rows(&[0.7, 0.3]);
        layout.remove_tab(A);
        assert_eq!(layout.rows.iter().map(|r| r.size).collect::<Vec<_>>(), [1.0]);

        layout.split_right(C, None);
        layout.split_right(D, None);
        layout.resize_cells(layout.rows[0].id, &[0.6, 0.3, 0.1]);
        layout.remove_tab(B);
        let sizes: Vec<f32> = layout.rows[0].cells.iter().map(|c| c.size).collect();
        assert!(sizes.iter().all(|size| (size - 0.5).abs() < 1e-6), "{sizes:?}");
    }

    #[test]
    fn a_saved_grid_that_does_not_fill_is_stretched_to_fill() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.resize_rows(&[0.3]);
        let restored = Layout::from_shape(&layout.shape(), &[A]);
        assert_eq!(restored.rows[0].size, 1.0);
    }

    #[test]
    fn a_dragged_tab_splits_a_cell_on_the_side_it_lands() {
        // B dropped on A's right edge: it moves there, and its old pane goes.
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.split_down(B, None);
        let first = layout.rows[0].cells[0].id;
        layout.split_with(B, first, Direction::Right);
        assert_eq!(shape(&layout), [[[1], [2]]]);
        assert_eq!(layout.focused_tab(), Some(B));
        let sizes: Vec<f32> = layout.rows[0].cells.iter().map(|c| c.size).collect();
        assert!((sizes.iter().sum::<f32>() - 1.0).abs() < 1e-6);

        // Dropping B on A's left edge puts it back on the left; B's old cell goes.
        layout.split_with(B, first, Direction::Left);
        assert_eq!(shape(&layout), [[[2], [1]]]);

        // A new row below A, from B.
        layout.split_with(B, first, Direction::Down);
        assert_eq!(shape(&layout), vec![vec![vec![1]], vec![vec![2]]]);
        assert!((layout.rows.iter().map(|r| r.size).sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_lone_tab_cannot_split_off_from_itself() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        let only = layout.rows[0].cells[0].id;
        layout.split_with(A, only, Direction::Right);
        assert_eq!(shape(&layout), [[[1]]]);
    }

    #[test]
    fn drops_an_emptied_cell_and_row_and_focuses_the_neighbour() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.split_right(B, None);
        layout.split_down(C, None);
        layout.remove_tab(C);
        assert_eq!(shape(&layout), [[[1], [2]]]);
        assert_eq!(layout.focused_tab(), Some(B));
    }

    #[test]
    fn empties_the_grid_when_the_last_tab_closes() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.remove_tab(A);
        assert!(layout.is_empty());
        assert_eq!(layout.focused, None);
    }

    #[test]
    fn clears_maximise_when_the_maximised_cell_goes() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.split_right(B, None);
        layout.toggle_maximize();
        layout.remove_tab(B);
        assert_eq!(layout.maximized, None);
    }

    #[test]
    fn a_pane_dropped_on_another_trades_places_with_it() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.split_right(B, None);
        layout.split_down(C, None);
        let target = cell_of(&layout, C);
        layout.swap(A, target);
        assert_eq!(shape(&layout), vec![vec![vec![3], vec![2]], vec![vec![1]]]);
        assert_eq!(layout.focused_tab(), Some(A));
    }

    #[test]
    fn keeps_every_tab_exactly_once() {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.split_right(B, None);
        layout.split_down(C, None);
        layout.add_tab(D, None);
        let target = cell_of(&layout, D);
        layout.swap(B, target);
        let mut tabs: Vec<u64> = layout.tabs().map(|t| t.0).collect();
        tabs.sort_unstable();
        assert_eq!(tabs, [1, 2, 3, 4]);
    }

    // a | b | c
    //   d   | e
    fn grid() -> Layout {
        let mut layout = Layout::default();
        layout.add_tab(A, None);
        layout.split_right(B, None);
        layout.split_right(C, None);
        layout.split_down(D, None);
        layout.split_right(E, None);
        layout.focus_nth(1);
        layout
    }

    #[test]
    fn counts_cells_in_reading_order() {
        let layout = grid();
        assert_eq!(layout.cells().map(|c| c.active.0).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
        let mut fourth = grid();
        fourth.focus_nth(4);
        assert_eq!(fourth.focused_tab(), Some(D));
        let mut ninth = grid();
        ninth.focus_nth(9);
        assert_eq!(ninth, layout);
    }

    #[test]
    fn moves_left_and_right_within_a_row_and_stops_at_the_edge() {
        let mut layout = grid();
        layout.focus_direction(Direction::Right);
        assert_eq!(layout.focused_tab(), Some(B));
        let mut edge = grid();
        edge.focus_direction(Direction::Left);
        assert_eq!(edge, grid());
    }

    #[test]
    fn moves_up_and_down_to_the_cell_most_directly_across() {
        let mut layout = grid();
        layout.focus_direction(Direction::Down);
        assert_eq!(layout.focused_tab(), Some(D));
        layout.focus_nth(3);
        layout.focus_direction(Direction::Down);
        assert_eq!(layout.focused_tab(), Some(E));
        layout.focus_nth(4);
        layout.focus_direction(Direction::Up);
        assert_eq!(layout.focused_tab(), Some(A));
    }

    #[test]
    fn cycles_through_the_panes() {
        let mut layout = grid();
        layout.cycle_pane(false);
        assert_eq!(layout.focused_tab(), Some(E));
        layout.cycle_pane(true);
        assert_eq!(layout.focused_tab(), Some(A));
        layout.cycle_pane(true);
        assert_eq!(layout.focused_tab(), Some(B));
    }

    #[test]
    fn shows_only_the_maximised_cell() {
        let mut layout = grid();
        assert_eq!(layout.visible_tabs(), [A, B, C, D, E]);
        layout.focus_nth(2);
        layout.toggle_maximize();
        assert_eq!(layout.visible_tabs(), [B]);
    }

    #[test]
    fn a_grid_survives_its_shape() {
        let mut layout = Layout::default();
        layout.add_tab(TabId(1), None);
        layout.add_tab(TabId(2), None);
        layout.split_right(TabId(3), None);
        layout.split_down(TabId(4), None);
        layout.activate_tab(TabId(1));
        let shape = layout.shape();
        assert_eq!(shape.tab_count(), 4);
        let restored = Layout::from_shape(&shape, &[TabId(11), TabId(12), TabId(13), TabId(14)]);
        assert_eq!(restored.shape(), shape);
        assert_eq!(restored.focused_tab(), Some(TabId(11)), "the focused cell and its front tab come back");
    }

    #[test]
    fn too_few_tabs_drop_the_cells_left_empty() {
        let mut layout = Layout::default();
        layout.add_tab(TabId(1), None);
        layout.split_right(TabId(2), None);
        let restored = Layout::from_shape(&layout.shape(), &[TabId(9)]);
        assert_eq!(restored.tabs().collect::<Vec<_>>(), [TabId(9)]);
        assert_eq!(restored.rows[0].cells.len(), 1);
    }
}
