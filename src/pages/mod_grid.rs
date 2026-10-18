//! The mod card grids of the Search, Details, Current modpack, shared pack and
//! mod install pages: how many columns a width gets, after each grid's
//! Tailwind classes in the desktop app, and a virtualized list whose items
//! are rows of cards.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    hash::{DefaultHasher, Hash as _, Hasher as _},
    ops::Range,
    rc::Rc,
};

use gpui::{
    AnyElement, EntityId, IntoElement, ListAlignment, ListOffset, ListState, ParentElement, Pixels,
    Styled, Window, canvas, div, px,
};

use crate::theme;

/// Whether the window reaches a Tailwind breakpoint (`md` 48, `lg` 64, `xl`
/// 80, `2xl` 96), in the desktop app's rem.
pub fn at_least(window_width: Pixels, rems: f32) -> bool {
    window_width >= px(rems * theme::FONT_SIZE)
}

/// `grid-cols-1 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4`: a modpack's
/// mods in `ModpackView` and `SharedModpackView`.
pub fn modpack_columns(window_width: Pixels) -> usize {
    match () {
        _ if at_least(window_width, 96.) => 4,
        _ if at_least(window_width, 80.) => 3,
        _ if at_least(window_width, 48.) => 2,
        _ => 1,
    }
}

/// `grid-cols-3 2xl:grid-cols-4`: the update check's results.
pub fn update_columns(window_width: Pixels) -> usize {
    if at_least(window_width, 96.) { 4 } else { 3 }
}

/// `md:grid-cols-2 2xl:grid-cols-3`: a mod's dependencies on its install
/// page.
pub fn dependency_columns(window_width: Pixels) -> usize {
    match () {
        _ if at_least(window_width, 96.) => 3,
        _ if at_least(window_width, 48.) => 2,
        _ => 1,
    }
}

/// `grid-cols-2 md:grid-cols-3 xl:grid-cols-4`: a mod's screenshots.
pub fn screenshot_columns(window_width: Pixels) -> usize {
    match () {
        _ if at_least(window_width, 80.) => 4,
        _ if at_least(window_width, 48.) => 3,
        _ => 2,
    }
}

/// The smallest search result card, `minmax(300px, 1fr)`.
pub const SEARCH_CARD_MIN: Pixels = px(300.);

/// `grid-cols-[repeat(auto-fill,minmax(min,1fr))]` with `gap` between
/// columns: as many columns as fit in `width`, at least one.
pub fn auto_fill_columns(width: Pixels, min: Pixels, gap: Pixels) -> usize {
    (((width + gap) / (min + gap)).floor() as usize).max(1)
}

/// `count` cards, `columns` to a row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rows {
    pub count: usize,
    pub columns: usize,
}

impl Rows {
    pub fn new(count: usize, columns: usize) -> Self {
        Self {
            count,
            columns: columns.max(1),
        }
    }

    pub fn len(self) -> usize {
        self.count.div_ceil(self.columns.max(1))
    }

    /// The cards on row `row`.
    pub fn cards(self, row: usize) -> Range<usize> {
        let start = (row * self.columns).min(self.count);
        start..(start + self.columns).min(self.count)
    }

    /// The row card `card` sits on.
    fn row_of(self, card: usize) -> usize {
        card / self.columns.max(1)
    }
}

/// The items of a virtualized page: `headers` items before the grid's rows,
/// then `trailers` after them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shape {
    pub headers: usize,
    pub rows: Rows,
    pub trailers: usize,
}

/// What item `ix` of a [`Shape`] is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Header(usize),
    Row { row: usize, cards: Range<usize> },
    Trailer(usize),
}

impl Shape {
    pub fn items(self) -> usize {
        self.headers + self.rows.len() + self.trailers
    }

    pub fn item(self, ix: usize) -> Item {
        if ix < self.headers {
            return Item::Header(ix);
        }
        let row = ix - self.headers;
        if row < self.rows.len() {
            return Item::Row {
                row,
                cards: self.rows.cards(row),
            };
        }
        Item::Trailer(row - self.rows.len())
    }

    /// Where `top` of `old` lands in `self`: the same header, the row
    /// holding the same first card, or the same trailer.
    fn carry(self, old: Shape, top: ListOffset) -> ListOffset {
        if top.item_ix >= old.items() {
            return ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            };
        }
        let item_ix = match old.item(top.item_ix) {
            Item::Header(header) => header.min(self.headers),
            Item::Row { cards, .. } => self.headers + self.rows.row_of(cards.start),
            Item::Trailer(trailer) => self.headers + self.rows.len() + trailer,
        };
        let same_place = self.rows.columns == old.rows.columns || item_ix < self.headers;
        ListOffset {
            item_ix: item_ix.min(self.items().saturating_sub(1)),
            offset_in_item: if same_place {
                top.offset_in_item
            } else {
                px(0.)
            },
        }
    }
}

/// A list of headers, rows of cards and trailers that keeps its scroll
/// position, by card, as the cards or the columns change.
pub struct GridList {
    pub state: ListState,
    shape: Shape,
    /// Each item's height until it is measured, so the scrollbar is about
    /// right from the start.
    hint: Pixels,
}

impl GridList {
    pub fn new(hint: Pixels) -> Self {
        Self {
            state: ListState::new(0, ListAlignment::Top, px(0.)),
            shape: Shape::default(),
            hint,
        }
    }

    pub fn shape(&self) -> Shape {
        self.shape
    }

    /// Re-shapes the list for this frame's items.
    pub fn set_shape(&mut self, shape: Shape) {
        if shape == self.shape {
            return;
        }
        let top = self.state.logical_scroll_top();
        let carried = shape.carry(self.shape, top);
        self.state
            .reset_with_uniform_height(shape.items(), self.hint);
        self.state.scroll_to(carried);
        self.shape = shape;
    }
}

/// Records the width a grid gets each frame and redraws `view` once more
/// when that width changes its column count, so a grid whose columns depend
/// on its own width follows the window without a frame's lag.
#[derive(Clone, Default)]
pub struct WidthProbe(Rc<Cell<Pixels>>);

impl WidthProbe {
    /// The width the grid got in the last frame; zero before the first.
    pub fn width(&self) -> Pixels {
        self.0.get()
    }

    /// An invisible element to lay over the grid. `columns` turns a width
    /// into the grid's column count.
    pub fn element(&self, view: EntityId, columns: fn(Pixels) -> usize) -> AnyElement {
        let width = self.0.clone();
        canvas(
            move |bounds, window: &mut Window, _| {
                let before = width.replace(bounds.size.width);
                if columns(before) != columns(bounds.size.width) {
                    window.on_next_frame(move |_, cx| cx.notify(view));
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
        .into_any_element()
    }
}

/// The heights rows of cards had when last laid out from their content, by
/// a key naming everything a row's height depends on. Sizing a grid row from
/// its cards' content lays each card out several times over; a row whose
/// height is known hands it to its cards, which then lay out once, to the
/// same result. Rows not drawn for two frames are forgotten.
#[derive(Clone, Default)]
pub struct RowHeights(Rc<RefCell<Heights>>);

#[derive(Default)]
struct Heights {
    /// The window width this frame, part of every key.
    width: Pixels,
    current: HashMap<u64, Pixels>,
    previous: HashMap<u64, Pixels>,
}

impl RowHeights {
    /// Starts a frame drawn at `window_width`.
    pub fn next_frame(&self, window_width: Pixels) {
        let heights = &mut *self.0.borrow_mut();
        heights.width = window_width;
        heights.previous = std::mem::take(&mut heights.current);
    }

    /// The key of a row of `columns` columns holding cards with these
    /// `card_keys` (see `mod_ui::card_key`), plus anything else in it.
    pub fn key(&self, columns: usize, card_keys: impl IntoIterator<Item = u64>, extra: u64) -> u64 {
        let mut hasher = DefaultHasher::new();
        f32::from(self.0.borrow().width).to_bits().hash(&mut hasher);
        (columns, extra).hash(&mut hasher);
        for key in card_keys {
            key.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Gives `element` the height known under `key`; until it is known,
    /// lays it out from its content and, when it is the one to `measure`,
    /// records the height it gets.
    pub fn apply<E: Styled + ParentElement>(&self, key: u64, measure: bool, element: E) -> E {
        match self.known(key) {
            Some(height) => element.h(height),
            None if measure => element.child(self.measure(key)),
            None => element,
        }
    }

    /// The height of the row under `key`, kept for the next frame too.
    fn known(&self, key: u64) -> Option<Pixels> {
        let heights = &mut *self.0.borrow_mut();
        let known = heights
            .current
            .get(&key)
            .or_else(|| heights.previous.get(&key))
            .copied()?;
        heights.current.insert(key, known);
        Some(known)
    }

    fn measure(&self, key: u64) -> AnyElement {
        let heights = self.0.clone();
        canvas(
            move |bounds, _, _| {
                heights.borrow_mut().current.insert(key, bounds.size.height);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
        .into_any_element()
    }
}

/// Where a card sits in a grid whose row heights are kept: its row's key,
/// and whether it is the row's first card, which measures the row.
#[derive(Clone, Copy)]
pub struct CardSlot<'a> {
    pub heights: &'a RowHeights,
    pub row: u64,
    pub first: bool,
}

impl CardSlot<'_> {
    /// The key of a part of the card `card_key` names, inside this row.
    pub fn part(&self, card_key: u64, part: u64) -> u64 {
        let mut hasher = DefaultHasher::new();
        (self.row, card_key, part).hash(&mut hasher);
        hasher.finish()
    }
}

/// One row of a grid: `cards` side by side in `columns` equal columns.
pub fn row(
    columns: usize,
    gap: impl Into<gpui::DefiniteLength>,
    cards: Vec<AnyElement>,
) -> gpui::Div {
    div()
        .grid()
        .grid_cols(columns as u16)
        .gap(gap.into())
        .children(cards)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoints_follow_the_desktop_app_rem() {
        assert_eq!(modpack_columns(px(800.)), 1);
        assert_eq!(modpack_columns(px(806.4)), 2);
        assert_eq!(modpack_columns(px(1344.)), 3);
        assert_eq!(modpack_columns(px(1613.)), 4);
        assert_eq!(update_columns(px(900.)), 3);
        assert_eq!(update_columns(px(2200.)), 4);
        assert_eq!(dependency_columns(px(900.)), 2);
        assert_eq!(screenshot_columns(px(700.)), 2);
    }

    #[test]
    fn auto_fill_fits_whole_columns() {
        let gap = px(12.6);
        assert_eq!(auto_fill_columns(px(0.), SEARCH_CARD_MIN, gap), 1);
        assert_eq!(auto_fill_columns(px(612.), SEARCH_CARD_MIN, gap), 1);
        assert_eq!(auto_fill_columns(px(613.), SEARCH_CARD_MIN, gap), 2);
        assert_eq!(auto_fill_columns(px(1000.), SEARCH_CARD_MIN, gap), 3);
    }

    #[test]
    fn rows_split_the_cards() {
        let rows = Rows::new(7, 3);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.cards(0), 0..3);
        assert_eq!(rows.cards(2), 6..7);
        assert_eq!(Rows::new(0, 3).len(), 0);
    }

    #[test]
    fn row_heights_follow_the_window_and_forget_rows_left_undrawn() {
        let heights = RowHeights::default();
        heights.next_frame(px(1000.));
        let key = heights.key(3, [1, 2, 3], 0);
        assert_ne!(key, heights.key(3, [1, 2, 4], 0));
        assert_ne!(key, heights.key(4, [1, 2, 3], 0));
        assert_eq!(heights.known(key), None);
        heights.0.borrow_mut().current.insert(key, px(190.));

        heights.next_frame(px(1000.));
        assert_eq!(heights.key(3, [1, 2, 3], 0), key);
        assert_eq!(heights.known(key), Some(px(190.)));
        heights.next_frame(px(1200.));
        assert_ne!(heights.key(3, [1, 2, 3], 0), key, "another width");

        heights.next_frame(px(1000.));
        heights.next_frame(px(1000.));
        assert_eq!(heights.known(key), None);
    }

    #[test]
    fn a_new_column_count_keeps_the_first_visible_card() {
        let old = Shape {
            headers: 1,
            rows: Rows::new(100, 2),
            trailers: 1,
        };
        let new = Shape {
            rows: Rows::new(100, 4),
            ..old
        };
        let top = ListOffset {
            item_ix: 11,
            offset_in_item: px(30.),
        };
        // Row 10 of two starts at card 20, which is on row 5 of four.
        let carried = new.carry(old, top);
        assert_eq!((carried.item_ix, carried.offset_in_item), (6, px(0.)));
        let kept = old.carry(old, top);
        assert_eq!((kept.item_ix, kept.offset_in_item), (11, px(30.)));
        assert_eq!(new.item(26), Item::Trailer(0));
        // A list's first shape starts at its top.
        let first = old.carry(Shape::default(), ListOffset::default());
        assert_eq!((first.item_ix, first.offset_in_item), (0, px(0.)));
    }
}
