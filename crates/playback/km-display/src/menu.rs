//! The window's Menu button and its two items, as geometry.
//!
//! **Pure layout, like [`crate::keypad`].** Nothing here opens a font or touches SDL, so the tests
//! run on a machine with no display. `draw` reads a [`WindowMenu`] and paints it, and the caller
//! hit-tests the same value, so what is drawn and what is pressed cannot disagree.
//!
//! **Top centre, where a phone's Menu button is.** The playing screen writes the song's title into
//! the top left and the badges into the top right. The middle of the top edge is the one place a
//! button covers neither. See `On Windows and macOS, the window has a Menu` in
//! `docs/decisions/remotes.md`.

/// One of the Menu's items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    /// Open the singer's remote, in the browser `F11` opens.
    ChooseSongs,
    /// Pick a `.kmpkg` in the system file browser and install it.
    InstallPackage,
}

impl MenuItem {
    /// The message id of the item's label, in this crate's catalog.
    pub fn label(self) -> &'static str {
        match self {
            Self::ChooseSongs => crate::words::MENU_CHOOSE_SONGS,
            Self::InstallPackage => crate::words::MENU_INSTALL_PACKAGE,
        }
    }
}

/// What a press on the Menu landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuHit {
    /// The Menu button itself, which opens or closes the list.
    Button,
    /// One of the items.
    Item(MenuItem),
}

/// A rectangle in backbuffer pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuRect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl MenuRect {
    /// Whether a point is inside. Half-open on the far edges, as [`crate::Key::contains`] is.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// The center, where a label goes.
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// The button's height as a share of the screen's height.
const BUTTON_SHARE: f32 = 0.05;

/// The smallest button height, in pixels, so a small window still gets a target a mouse can hit.
const MIN_BUTTON_PX: f32 = 28.0;

/// The button's width in button heights. Wide enough for *Menu* in either language.
const BUTTON_WIDTH: f32 = 3.0;

/// An item's width in button heights. Wide enough for *Instalar um pacote* in the small face.
const ITEM_WIDTH: f32 = 7.0;

/// The Menu as it stands this frame: the button, and the items when it is open.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowMenu {
    button: MenuRect,
    items: Vec<(MenuItem, MenuRect)>,
    hover: Option<MenuHit>,
}

impl WindowMenu {
    /// Lays the Menu out on a screen of this size.
    ///
    /// `open` draws the items under the button. `choose_songs` is false where the remote is turned
    /// off, and the item is then left out, as a phone leaves it out. `pointer` is where the mouse
    /// is, in backbuffer pixels, so the item under it can be drawn highlighted.
    pub fn new(
        width: f32,
        height: f32,
        open: bool,
        choose_songs: bool,
        pointer: Option<(f32, f32)>,
    ) -> Self {
        let h = (height * BUTTON_SHARE).max(MIN_BUTTON_PX);
        let gap = h * 0.2;
        let button = MenuRect {
            x: (width - h * BUTTON_WIDTH) / 2.0,
            y: gap,
            w: h * BUTTON_WIDTH,
            h,
        };
        let items = if open {
            let w = (h * ITEM_WIDTH).min(width);
            [MenuItem::ChooseSongs, MenuItem::InstallPackage]
                .into_iter()
                .filter(|item| choose_songs || *item != MenuItem::ChooseSongs)
                .enumerate()
                .map(|(row, item)| {
                    let rect = MenuRect {
                        x: (width - w) / 2.0,
                        y: button.y + h + gap + row as f32 * h,
                        w,
                        h,
                    };
                    (item, rect)
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut menu = Self {
            button,
            items,
            hover: None,
        };
        menu.hover = pointer.and_then(|(x, y)| menu.hit(x, y));
        menu
    }

    /// What a press at this point lands on, or `None` for a press outside the Menu.
    pub fn hit(&self, x: f32, y: f32) -> Option<MenuHit> {
        if self.button.contains(x, y) {
            return Some(MenuHit::Button);
        }
        self.items
            .iter()
            .find(|(_, rect)| rect.contains(x, y))
            .map(|(item, _)| MenuHit::Item(*item))
    }

    /// The button's rectangle.
    pub fn button(&self) -> MenuRect {
        self.button
    }

    /// The items and their rectangles, top first. Empty while the Menu is closed.
    pub fn items(&self) -> &[(MenuItem, MenuRect)] {
        &self.items
    }

    /// Whether the Menu is open.
    pub fn is_open(&self) -> bool {
        !self.items.is_empty()
    }

    /// What the mouse is over, which is drawn highlighted.
    pub fn hover(&self) -> Option<MenuHit> {
        self.hover
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_sits_at_the_top_centre() {
        let menu = WindowMenu::new(1920.0, 1080.0, false, true, None);
        let (x, _) = menu.button().center();
        assert!((x - 960.0).abs() < 0.5, "{x}");
        assert!(menu.button().y < 1080.0 * 0.1);
        assert!(menu.items().is_empty());
        assert!(!menu.is_open());
    }

    #[test]
    fn a_small_window_still_gets_a_button_a_mouse_can_hit() {
        let menu = WindowMenu::new(400.0, 300.0, false, true, None);
        assert!(menu.button().h >= MIN_BUTTON_PX);
    }

    #[test]
    fn an_open_menu_lists_both_items_under_the_button() {
        let menu = WindowMenu::new(1280.0, 720.0, true, true, None);
        let items: Vec<MenuItem> = menu.items().iter().map(|(item, _)| *item).collect();
        assert_eq!(items, [MenuItem::ChooseSongs, MenuItem::InstallPackage]);
        let button = menu.button();
        for (_, rect) in menu.items() {
            assert!(rect.y >= button.y + button.h, "an item covers the button");
        }
    }

    #[test]
    fn a_turned_off_remote_leaves_choose_songs_out() {
        let menu = WindowMenu::new(1280.0, 720.0, true, false, None);
        let items: Vec<MenuItem> = menu.items().iter().map(|(item, _)| *item).collect();
        assert_eq!(items, [MenuItem::InstallPackage]);
    }

    #[test]
    fn a_press_lands_on_what_is_drawn_there() {
        let menu = WindowMenu::new(1280.0, 720.0, true, true, None);
        let (x, y) = menu.button().center();
        assert_eq!(menu.hit(x, y), Some(MenuHit::Button));
        for (item, rect) in menu.items() {
            let (x, y) = rect.center();
            assert_eq!(menu.hit(x, y), Some(MenuHit::Item(*item)));
        }
        assert_eq!(menu.hit(5.0, 700.0), None);
    }

    #[test]
    fn a_closed_menu_has_no_items_to_press() {
        let open = WindowMenu::new(1280.0, 720.0, true, true, None);
        let (x, y) = open.items()[0].1.center();
        let closed = WindowMenu::new(1280.0, 720.0, false, true, None);
        assert_eq!(closed.hit(x, y), None);
    }

    #[test]
    fn the_item_under_the_mouse_is_the_one_highlighted() {
        let probe = WindowMenu::new(1280.0, 720.0, true, true, None);
        let at = probe.items()[1].1.center();
        let menu = WindowMenu::new(1280.0, 720.0, true, true, Some(at));
        assert_eq!(menu.hover(), Some(MenuHit::Item(MenuItem::InstallPackage)));
    }
}
