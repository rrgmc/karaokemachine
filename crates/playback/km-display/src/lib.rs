//! SDL3 karaoke display.
//!
//! Split so the parts that decide *what* to show are testable, and only the drawing needs a screen:
//!
//! * [`lyrics`] -- which lines belong on screen and how far the highlight has crossed them, from a
//!   playback tick. Pure logic.
//! * [`numbers`] -- the song-number keypad.
//! * [`keypad`] -- where the on-screen touch targets are, and what a touch at a point means. Pure
//!   geometry, because Android has no keyboard and a key that falls off the screen is a bug worth
//!   catching without one.
//! * [`wallpaper`] -- the playlist, the crossfade schedule, and a loader thread that decodes off
//!   the render thread.
//! * [`offscreen`] -- the same drawing, into an image instead of a window. What the preview contact
//!   sheet and the README's pictures are both made with.
//! * [`performance`] -- what the frame meter measured, and which of it is worth coloring. Pure
//!   logic, for the reason [`lyrics`] is: the machine measures, this decides what it means.
//!
//! See `docs/ARCHITECTURE.md`.

pub mod catalog;
pub mod connect;
pub mod draw;
pub mod icon;
pub mod input;
pub mod keypad;
pub mod lyrics;
pub mod numbers;
pub mod offscreen;
pub mod performance;
pub mod song_stats;
pub mod text;
pub mod theme;
pub mod wallpaper;
pub mod words;

pub use crate::catalog::CatalogSummary;
pub use crate::connect::{BrowserKey, ConnectInfo, ConnectPanel, ConnectProblem, QrMatrix};
pub use crate::draw::{
    Background, DeveloperMode, FaultArea, Faults, Flash, FlashKind, Frame, Screen, SongInfo,
    position_reserve, version_reserve,
};
pub use crate::icon::set_window_icon;
pub use crate::input::{
    Direction, DisplayAction, TRANSPORT_COMMANDS, TransportCommand, action_for,
};
pub use crate::keypad::{Key, Keypad, MIN_KEY_PX, pixel_density, window_to_pixels};
pub use crate::lyrics::{LyricFrame, LyricView, ROWS, VisibleLine};
pub use crate::numbers::{NumberAction, NumberEntry, SongPreview};
pub use crate::offscreen::{Backdrop, Offscreen, OffscreenError, render_to_image};
// **The picture type this crate's own surface names.** `Backdrop` and `Offscreen::set_picture`
// both take one, so a caller that draws over a photograph would otherwise have to declare `image`
// itself to say what it is handing over -- a dependency added to name a type, not to use a crate.
pub use crate::performance::FrameStats;
pub use crate::song_stats::{GainSource, SongMedia, SongStats};
pub use crate::text::{
    Align, Fonts, LineMetrics, TextError, TextStyle, WipeStyle, WipedLine, find_font, measure_line,
};
pub use crate::theme::Theme;
pub use crate::wallpaper::{
    Fit, ImageSource, Loader, Playlist, Schedule, Shuffle, WallpaperConfig,
};
pub use image::RgbaImage;
