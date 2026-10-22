//! The Skin Museum dialog (Skins > Skin Museum...): the Winamp Skin Museum's skins in a list
//! that a search narrows, with the highlighted skin's screenshot beside it. A moment after a
//! skin is highlighted, NeoAmp's own windows wear it, to try it on; Install keeps it in the
//! skins folder, and Close puts back the skin that was showing.
//!
//! This is the dialog's state, layout, input and drawing, drawn like Jump to file
//! ([`crate::jump`]). It fetches nothing itself: `museum_window` asks it what is wanted
//! ([`Museum::next_request`]), fetches that on threads of its own and hands back what arrives.
//! Geometry is in physical pixels: Windows' sizes at 96 DPI times the dialog's scale.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::canvas::{Image, Rect};
use crate::classic::{self, FACE, GRAY_TEXT, TEXT, line, px};
use crate::font::TtfFont;
use crate::list_box::ListBox;
use crate::menu::Measure;
use crate::museum::{Listing, MuseumSkin, Page};
use crate::text_box::{Edit, TextBox};

/// The window's title.
pub const TITLE: &str = "Skin Museum";
/// Typing waits this long for the next key before searching.
const SEARCH_DELAY: Duration = Duration::from_millis(400);
/// A highlighted skin's screenshot is fetched once the highlight has rested this long...
const SCREENSHOT_DELAY: Duration = Duration::from_millis(150);
/// ...and the skin itself, to try it on, after this long.
const TRY_DELAY: Duration = Duration::from_millis(500);
/// The next page is fetched while fewer rows than this are left below those shown.
const PREFETCH_ROWS: usize = 10;
/// Screenshots kept in memory; the cache folder keeps more.
const SCREENSHOTS_KEPT: usize = 32;

/// Sizes at 96 DPI, which `px` scales.
mod metric {
    pub const W: i32 = 600;
    pub const H: i32 = 420;
    pub const MARGIN: i32 = 8;
    pub const GAP: i32 = 6;
    pub const EDIT_H: i32 = 21;
    pub const BUTTON_W: i32 = 75;
    pub const BUTTON_H: i32 = 23;
    /// The museum's screenshots: the three windows stacked, at 1x.
    pub const SHOT_W: i32 = 275;
    pub const SHOT_H: i32 = 348;
    /// From the screenshot's box to the line under it.
    pub const CAPTION_GAP: i32 = 4;
    pub const LINE_H: i32 = 14;
}

const LABEL: &str = "Search:";
const INSTALL: &str = "Install";
const CLOSE: &str = "Close";

/// What a key does in the dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// Move the highlight in the list.
    Up,
    Down,
    PageUp,
    PageDown,
    /// Move the caret in the search box.
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    Enter,
    Escape,
}

/// What the window layer must do after an input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    None,
    Redraw,
    /// Keep the skin being worn, which is the highlighted one.
    Install,
    Close,
}

/// Something the dialog wants fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// The page of `listing` that starts at skin `offset`.
    Page {
        listing: Listing,
        offset: usize,
    },
    Screenshot(MuseumSkin),
    /// The skin itself, to try it on.
    Skin(MuseumSkin),
}

/// What to do with a skin that has arrived to be tried on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arrival {
    /// Nothing: the highlight has moved on, or it could not be used.
    Drop,
    /// Wear it.
    Wear,
    /// Wear it and keep it: Install was pressed while it was on its way.
    Install,
}

/// What the museum has listed, by listing. Kept while NeoAmp runs, so that reopening the
/// dialog or going back to an earlier search fetches nothing again.
#[derive(Debug, Default)]
pub struct Listings(HashMap<Listing, Listed>);

/// One listing, as far as it has been fetched.
#[derive(Debug)]
struct Listed {
    skins: Vec<MuseumSkin>,
    /// Where the next page starts.
    next: usize,
    /// The museum may have more.
    more: bool,
    /// A page has been asked for and has not arrived.
    loading: bool,
    /// Why the last page could not be fetched.
    failed: Option<String>,
}

impl Default for Listed {
    fn default() -> Self {
        Listed {
            skins: Vec::new(),
            next: 0,
            more: true,
            loading: false,
            failed: None,
        }
    }
}

/// A screenshot, as far as it has been fetched.
#[derive(Debug)]
enum Shot {
    Loading,
    Ready(Image),
    Failed,
}

/// How far trying a skin on has got.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Trying {
    Idle,
    /// The skin with this MD5 is on its way.
    Fetching(String),
    Wearing(String),
    /// Worn, and kept.
    Installed(String),
    /// It could not be used, for the reason given.
    Failed(String, String),
}

/// A part of the dialog the pointer can press.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Edit,
    /// The list, rows or scroll bar.
    List,
    InstallButton,
    CloseButton,
}

/// Where everything is, in the dialog's pixels.
#[derive(Clone, Copy, Debug)]
struct Layout {
    size: (i32, i32),
    /// One-pixel lines at this scale.
    t: i32,
    label_x: i32,
    edit: Rect,
    list: Rect,
    /// The line beside the buttons that says how the list is doing.
    status: Rect,
    /// The screenshot's sunken box.
    shot: Rect,
    /// The screenshot, inside the box.
    picture: Rect,
    /// The line under the screenshot.
    caption: Rect,
    install: Rect,
    close: Rect,
}

impl Layout {
    fn new(scale: f64, m: &dyn Measure) -> Layout {
        let p = |v| px(v, scale);
        let t = line(scale);
        let (w, h) = (p(metric::W), p(metric::H));
        let (margin, gap) = (p(metric::MARGIN), p(metric::GAP));
        let border = 2 * t;
        let picture_size = (p(metric::SHOT_W), p(metric::SHOT_H));
        let shot = Rect::new(
            w - margin - picture_size.0 - 2 * border,
            margin,
            picture_size.0 + 2 * border,
            picture_size.1 + 2 * border,
        );
        let picture = Rect::new(
            shot.x + border,
            shot.y + border,
            picture_size.0,
            picture_size.1,
        );
        let caption = Rect::new(
            shot.x,
            shot.bottom() + p(metric::CAPTION_GAP),
            shot.w,
            p(metric::LINE_H),
        );
        let left_w = shot.x - 2 * margin;
        let label_w = m.text_width(LABEL);
        let edit_x = margin + label_w + gap;
        let edit = Rect::new(edit_x, margin, margin + left_w - edit_x, p(metric::EDIT_H));
        let button = (p(metric::BUTTON_W), p(metric::BUTTON_H));
        let buttons_y = h - margin - button.1;
        let close = Rect::new(w - margin - button.0, buttons_y, button.0, button.1);
        let install = close.offset(-(button.0 + gap), 0);
        let list_y = edit.bottom() + gap;
        let list = Rect::new(margin, list_y, left_w, buttons_y - gap - list_y);
        let status = Rect::new(margin, buttons_y, left_w, button.1);
        Layout {
            size: (w, h),
            t,
            label_x: margin,
            edit,
            list,
            status,
            shot,
            picture,
            caption,
            install,
            close,
        }
    }
}

/// The dialog.
#[derive(Debug)]
pub struct Museum {
    scale: f64,
    layout: Layout,
    /// The search box.
    search: TextBox,
    /// When the search box last changed, while what it says is not yet listed.
    typed: Option<Instant>,
    listing: Listing,
    listings: Listings,
    /// One row per skin of `listing`.
    list: ListBox,
    /// The MD5 of the highlighted skin.
    highlight: Option<String>,
    /// When the highlight moved to that skin; `None` while nothing is to be fetched for it
    /// unasked.
    picked: Option<Instant>,
    shots: HashMap<String, Shot>,
    /// The screenshots in `shots`, oldest first.
    shot_order: VecDeque<String>,
    /// The highlighted skin's screenshot at the dialog's scale, with its MD5.
    picture: Option<(String, Image)>,
    trying: Trying,
    /// Install was pressed for the skin with this MD5 before it had arrived.
    install_wanted: Option<String>,
    pressed: Option<Part>,
    /// The last pointer position: a held button shows pushed in only while under it.
    pointer: (i32, i32),
}

impl Museum {
    /// A dialog at `scale` times Windows' 96 DPI sizes, measuring text with `m`, listing the
    /// museum's skins as far as `listings` has them.
    #[must_use]
    pub fn new(scale: f64, m: &dyn Measure, mut listings: Listings, now: Instant) -> Museum {
        // Whatever was on its way when the dialog last closed never arrived, and failures
        // are worth trying again.
        for listed in listings.0.values_mut() {
            listed.loading = false;
            listed.failed = None;
        }
        let layout = Layout::new(scale, m);
        let mut list = ListBox::new(layout.list, scale);
        let listing = Listing::Museum;
        list.reset(listings.0.get(&listing).map_or(0, |l| l.skins.len()), None);
        Museum {
            scale,
            layout,
            search: TextBox::new(now),
            typed: None,
            listing,
            listings,
            list,
            highlight: None,
            picked: None,
            shots: HashMap::new(),
            shot_order: VecDeque::new(),
            picture: None,
            trying: Trying::Idle,
            install_wanted: None,
            pressed: None,
            pointer: (-1, -1),
        }
    }

    /// What has been listed, to keep for the next time the dialog opens. The dialog lists
    /// nothing afterwards.
    pub fn take_listings(&mut self) -> Listings {
        self.list.reset(0, None);
        std::mem::take(&mut self.listings)
    }

    /// The window's size in pixels.
    #[must_use]
    pub fn size(&self) -> (i32, i32) {
        self.layout.size
    }

    #[must_use]
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Lay the dialog out again at another scale, keeping everything else.
    pub fn rescale(&mut self, scale: f64, m: &dyn Measure) {
        self.scale = scale;
        self.layout = Layout::new(scale, m);
        self.list.relayout(self.layout.list, scale);
        self.picture = None;
        self.update_picture();
    }

    #[must_use]
    pub fn query(&self) -> &str {
        self.search.text()
    }

    /// The skins listed.
    #[must_use]
    pub fn skins(&self) -> &[MuseumSkin] {
        self.listings
            .0
            .get(&self.listing)
            .map_or(&[], |l| l.skins.as_slice())
    }

    /// The highlighted skin.
    #[must_use]
    pub fn highlighted(&self) -> Option<&MuseumSkin> {
        self.skins().get(self.list.selected()?)
    }

    /// The highlight may have moved to another skin: its screenshot shows if it is to hand,
    /// and the wait to fetch it, and to try the skin on, starts again.
    fn sync_highlight(&mut self, now: Instant) {
        let md5 = self.highlighted().map(|s| s.md5.clone());
        if md5 == self.highlight {
            return;
        }
        self.highlight = md5;
        self.picked = self.highlight.is_some().then_some(now);
        self.install_wanted = None;
        self.update_picture();
    }

    /// Scale the highlighted skin's screenshot to the dialog, if it is to hand.
    fn update_picture(&mut self) {
        let Some(md5) = &self.highlight else {
            self.picture = None;
            return;
        };
        if self.picture.as_ref().is_some_and(|(m, _)| m == md5) {
            return;
        }
        let r = self.layout.picture;
        self.picture = match self.shots.get(md5) {
            Some(Shot::Ready(image)) => Some((md5.clone(), image.resized(r.w, r.h))),
            _ => None,
        };
    }

    fn keep_shot(&mut self, md5: String, shot: Shot) {
        if self.shots.insert(md5.clone(), shot).is_none() {
            self.shot_order.push_back(md5);
        }
        while self.shot_order.len() > SCREENSHOTS_KEPT {
            if let Some(old) = self.shot_order.pop_front() {
                self.shots.remove(&old);
            }
        }
    }

    /// List what the search box says now.
    fn search_now(&mut self) {
        self.typed = None;
        let listing = Listing::for_query(self.search.text());
        if listing == self.listing {
            return;
        }
        let len = self.listings.0.get(&listing).map_or(0, |l| l.skins.len());
        self.listing = listing;
        self.list.reset(len, None);
        self.highlight = None;
        self.picked = None;
        self.install_wanted = None;
        self.picture = None;
    }

    /// Call every frame: once typing has paused, list what the search box says. True if the
    /// dialog needs drawing again.
    pub fn tick(&mut self, now: Instant) -> bool {
        if self
            .typed
            .is_some_and(|t| now.saturating_duration_since(t) >= SEARCH_DELAY)
        {
            self.search_now();
            return true;
        }
        false
    }

    /// The next thing to fetch, if any, now marked as on its way: more of the listing while
    /// its end is near; then the highlighted skin's screenshot, and the skin itself to try on,
    /// once the highlight has rested a moment (or at once, after Install).
    pub fn next_request(&mut self, now: Instant) -> Option<Request> {
        let len = self.list.len();
        let near_end = self.list.scroll() + self.list.visible_rows() + PREFETCH_ROWS >= len
            || self
                .list
                .selected()
                .is_some_and(|s| s + PREFETCH_ROWS >= len);
        let listed = self.listings.0.entry(self.listing.clone()).or_default();
        if near_end && listed.more && !listed.loading && listed.failed.is_none() {
            listed.loading = true;
            return Some(Request::Page {
                listing: self.listing.clone(),
                offset: listed.next,
            });
        }
        let skin = self.highlighted()?.clone();
        let rested = self.picked.map(|t| now.saturating_duration_since(t));
        if rested.is_some_and(|r| r >= SCREENSHOT_DELAY) && !self.shots.contains_key(&skin.md5) {
            self.keep_shot(skin.md5.clone(), Shot::Loading);
            return Some(Request::Screenshot(skin));
        }
        let wanted = self.install_wanted.as_ref() == Some(&skin.md5)
            || rested.is_some_and(|r| r >= TRY_DELAY);
        let busy = match &self.trying {
            Trying::Idle => false,
            Trying::Fetching(m) | Trying::Wearing(m) | Trying::Installed(m) => *m == skin.md5,
            // Tried and failed: only Install tries again.
            Trying::Failed(m, _) => *m == skin.md5 && self.install_wanted.is_none(),
        };
        if wanted && !busy {
            self.trying = Trying::Fetching(skin.md5.clone());
            return Some(Request::Skin(skin));
        }
        None
    }

    /// A page asked for has arrived, or could not be fetched. True if the dialog needs drawing
    /// again.
    pub fn page_arrived(
        &mut self,
        listing: &Listing,
        offset: usize,
        page: Result<Page, String>,
    ) -> bool {
        let Some(listed) = self.listings.0.get_mut(listing) else {
            return false;
        };
        if !listed.loading || listed.next != offset {
            // Asked for before the dialog last closed.
            return false;
        }
        listed.loading = false;
        match page {
            Ok(page) => {
                listed.more = page.has_more();
                listed.next += page.sent;
                listed.skins.extend(page.skins);
            }
            Err(why) => {
                log::warn!("cannot list the Skin Museum's skins: {why}");
                listed.failed = Some(why);
            }
        }
        let len = listed.skins.len();
        if *listing != self.listing {
            return false;
        }
        self.list.set_len(len);
        true
    }

    /// A screenshot asked for has arrived, or could not be fetched. True if the dialog needs
    /// drawing again.
    pub fn screenshot_arrived(&mut self, md5: &str, picture: Result<Image, String>) -> bool {
        let shot = match picture {
            Ok(image) => Shot::Ready(image),
            Err(why) => {
                log::warn!("cannot show the Skin Museum's picture of {md5}: {why}");
                Shot::Failed
            }
        };
        self.keep_shot(md5.to_owned(), shot);
        if self.highlight.as_deref() != Some(md5) {
            return false;
        }
        self.update_picture();
        true
    }

    /// A skin asked for to try on has arrived (`Ok`), or could not be fetched or read: what
    /// to do with it.
    pub fn skin_arrived(&mut self, md5: &str, skin: Result<(), String>) -> Arrival {
        if self.trying != Trying::Fetching(md5.to_owned()) {
            return Arrival::Drop;
        }
        if self.highlight.as_deref() != Some(md5) {
            self.trying = Trying::Idle;
            return Arrival::Drop;
        }
        match skin {
            Ok(()) => {
                if self.install_wanted.take_if(|m| m == md5).is_some() {
                    self.trying = Trying::Installed(md5.to_owned());
                    Arrival::Install
                } else {
                    self.trying = Trying::Wearing(md5.to_owned());
                    Arrival::Wear
                }
            }
            Err(why) => {
                log::warn!("cannot try on the Skin Museum's skin {md5}: {why}");
                self.trying = Trying::Failed(md5.to_owned(), why);
                self.install_wanted = None;
                Arrival::Drop
            }
        }
    }

    /// NeoAmp no longer wears the skin being tried on (another one was chosen): leave the
    /// highlighted skin alone until the highlight moves again.
    pub fn dropped(&mut self) {
        self.trying = Trying::Idle;
        self.picked = None;
        self.install_wanted = None;
    }

    /// Install (or Enter): keep the highlighted skin, once it is worn.
    fn install(&mut self) -> Outcome {
        let Some(md5) = self.highlight.clone() else {
            return Outcome::None;
        };
        match &self.trying {
            Trying::Wearing(m) if *m == md5 => {
                self.trying = Trying::Installed(md5);
                Outcome::Install
            }
            Trying::Installed(m) if *m == md5 => Outcome::None,
            _ => {
                // Fetched (or fetched again) at once, and kept when it arrives.
                self.install_wanted = Some(md5);
                Outcome::Redraw
            }
        }
    }

    /// The text changed: list what it says once typing pauses.
    fn edited(&mut self, now: Instant) {
        self.typed = Some(now);
    }

    /// Typed or pasted text (typed text comes from the keyboard's layout, so already shifted
    /// or composed).
    pub fn text(&mut self, text: &str, now: Instant) -> Outcome {
        if !self.search.insert(text, now) {
            return Outcome::None;
        }
        self.edited(now);
        Outcome::Redraw
    }

    /// Select everything in the search box (Ctrl+A), so that typing replaces it.
    pub fn select_all(&mut self, now: Instant) -> Outcome {
        self.search.select_all(now);
        Outcome::Redraw
    }

    /// A key that is not text.
    pub fn key(&mut self, key: Key, now: Instant) -> Outcome {
        let rows = match key {
            Key::Up => Some(-1),
            Key::Down => Some(1),
            Key::PageUp => Some(-self.list.page()),
            Key::PageDown => Some(self.list.page()),
            _ => None,
        };
        if let Some(rows) = rows {
            self.list.move_selection(rows);
            self.sync_highlight(now);
            return Outcome::Redraw;
        }
        let edit = match key {
            // A search typed and not yet listed is listed at once; otherwise Enter installs.
            Key::Enter if self.typed.is_some() => {
                self.search_now();
                return Outcome::Redraw;
            }
            Key::Enter => return self.install(),
            Key::Escape => return Outcome::Close,
            Key::Left => Edit::Left,
            Key::Right => Edit::Right,
            Key::Home => Edit::Home,
            Key::End => Edit::End,
            Key::Backspace => Edit::Backspace,
            Key::Delete => Edit::Delete,
            Key::Up | Key::Down | Key::PageUp | Key::PageDown => return Outcome::None,
        };
        let had = (self.search.text().len(), self.search.is_all_selected());
        if self.search.edit(edit, now) {
            self.edited(now);
        } else if matches!(edit, Edit::Backspace | Edit::Delete)
            && had == (self.search.text().len(), false)
        {
            // Nothing to delete.
            return Outcome::None;
        }
        Outcome::Redraw
    }

    /// The mouse wheel turned `notches`, positive away from the user.
    pub fn wheel(&mut self, notches: i32) -> Outcome {
        self.list.wheel(notches);
        Outcome::Redraw
    }

    fn part_at(&self, x: i32, y: i32) -> Option<Part> {
        let l = &self.layout;
        if l.edit.contains(x, y) {
            return Some(Part::Edit);
        }
        if l.install.contains(x, y) {
            return Some(Part::InstallButton);
        }
        if l.close.contains(x, y) {
            return Some(Part::CloseButton);
        }
        self.list.part_at(x, y).map(|_| Part::List)
    }

    /// The pointer pressed at (`x`, `y`).
    pub fn pointer_down(&mut self, x: i32, y: i32, now: Instant, m: &dyn Measure) -> Outcome {
        self.pointer = (x, y);
        let Some(part) = self.part_at(x, y) else {
            return Outcome::None;
        };
        match part {
            Part::Edit => self.search.click(x, self.layout.edit, self.scale, m, now),
            Part::List => {
                self.list.pointer_down(x, y);
                self.sync_highlight(now);
            }
            Part::InstallButton | Part::CloseButton => {}
        }
        self.pressed = Some(part);
        Outcome::Redraw
    }

    /// Whether `part` is held with the pointer over it, so that letting go would press it.
    fn pushed(&self, part: Part) -> bool {
        self.pressed == Some(part) && self.part_at(self.pointer.0, self.pointer.1) == Some(part)
    }

    /// The pointer moved to (`x`, `y`): drags the scroll bar's thumb, and a held button pops
    /// out while the pointer is off it.
    pub fn pointer_moved(&mut self, x: i32, y: i32) -> Outcome {
        let held = self
            .pressed
            .filter(|p| matches!(p, Part::InstallButton | Part::CloseButton));
        if let Some(button) = held {
            let was = self.pushed(button);
            self.pointer = (x, y);
            return if self.pushed(button) == was {
                Outcome::None
            } else {
                Outcome::Redraw
            };
        }
        self.pointer = (x, y);
        if self.pressed == Some(Part::List) && self.list.pointer_moved(y) {
            Outcome::Redraw
        } else {
            Outcome::None
        }
    }

    /// The pointer let go at (`x`, `y`): a button pressed and let go over runs.
    pub fn pointer_up(&mut self, x: i32, y: i32) -> Outcome {
        let Some(part) = self.pressed.take() else {
            return Outcome::None;
        };
        let over = self.part_at(x, y) == Some(part);
        self.list.pointer_up();
        match part {
            Part::InstallButton if over => match self.install() {
                Outcome::None => Outcome::Redraw,
                outcome => outcome,
            },
            Part::CloseButton if over => Outcome::Close,
            _ => Outcome::Redraw,
        }
    }

    /// Whether the caret shows at `now`.
    #[must_use]
    pub fn caret_on(&self, now: Instant) -> bool {
        self.search.caret_on(now)
    }

    /// The line beside the buttons: how the list is doing.
    fn status(&self) -> String {
        let searching = matches!(self.listing, Listing::Search(_));
        let Some(listed) = self.listings.0.get(&self.listing) else {
            return String::new();
        };
        let n = listed.skins.len();
        if n == 0 {
            return if listed.loading {
                if searching {
                    "Searching..."
                } else {
                    "Loading..."
                }
                .to_owned()
            } else if listed.failed.is_some() {
                "Can't reach the museum".to_owned()
            } else if listed.more {
                String::new()
            } else {
                "No skins found".to_owned()
            };
        }
        let plus = if listed.more { "+" } else { "" };
        let s = if n == 1 && !listed.more { "" } else { "s" };
        let found = if searching { " found" } else { "" };
        let mut status = format!("{n}{plus} skin{s}{found}");
        if listed.failed.is_some() {
            status.push_str("; can't fetch more");
        }
        status
    }

    /// The line under the screenshot: the highlighted skin, and whether it is worn.
    fn caption(&self) -> String {
        let Some(skin) = self.highlighted() else {
            return String::new();
        };
        let name = skin.name();
        let this = |m: &String| *m == skin.md5;
        match &self.trying {
            Trying::Fetching(m) if this(m) => format!("Fetching {name}..."),
            Trying::Wearing(m) if this(m) => format!("Trying on {name}"),
            Trying::Installed(m) if this(m) => format!("Installed {name}"),
            Trying::Failed(m, _) if this(m) => format!("Can't use {name}"),
            _ => name.to_owned(),
        }
    }

    /// What the screenshot's box says while it shows no screenshot.
    fn placeholder(&self) -> &'static str {
        let Some(md5) = &self.highlight else {
            return "Choose a skin to try it on";
        };
        match self.shots.get(md5) {
            Some(Shot::Failed) => "No picture of this skin",
            _ => "Loading the picture...",
        }
    }

    /// Draw the dialog into `c`, which must be [`Museum::size`]. `font` must be the menu font
    /// at this scale ([`crate::menu::font_px`]).
    pub fn draw(&self, c: &mut Image, font: &TtfFont, now: Instant) {
        let l = &self.layout;
        let t = l.t;
        let lh = font.line_height();
        c.clear(FACE);
        let all = c.bounds();
        let centred = |r: Rect| r.y + (r.h - lh) / 2;

        font.draw(c, LABEL, l.label_x, centred(l.edit), TEXT, all);
        self.search.draw(c, font, l.edit, t, self.scale, now);

        let skins = self.skins();
        self.list.draw(c, font, |row, label| {
            let Some(skin) = skins.get(row) else {
                return false;
            };
            label.push_str(skin.name());
            true
        });
        font.draw(
            c,
            &self.status(),
            l.status.x,
            centred(l.status),
            TEXT,
            l.status,
        );

        classic::sunken(c, l.shot, t);
        match &self.picture {
            Some((md5, image)) if self.highlight.as_ref() == Some(md5) => {
                c.blit(image, image.bounds(), l.picture.x, l.picture.y);
            }
            _ => {
                let text = self.placeholder();
                let x = l.picture.x + (l.picture.w - font.width(text)) / 2;
                font.draw(c, text, x, centred(l.picture), GRAY_TEXT, l.picture);
            }
        }
        let caption = self.caption();
        let x = l.caption.x + (l.caption.w - font.width(&caption)).max(0) / 2;
        font.draw(c, &caption, x, centred(l.caption), TEXT, l.caption);

        // The buttons; Install is the default, which Enter presses.
        for (part, r, text) in [
            (Part::InstallButton, l.install, INSTALL),
            (Part::CloseButton, l.close, CLOSE),
        ] {
            let default = part == Part::InstallButton;
            classic::push_button(c, font, r, t, text, default, self.pushed(part));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Every character 6 pixels wide.
    struct Fixed;

    impl Measure for Fixed {
        fn text_width(&self, text: &str) -> i32 {
            i32::try_from(text.chars().count()).unwrap() * 6
        }
    }

    fn skin(i: usize) -> MuseumSkin {
        MuseumSkin {
            md5: format!("{i:032x}"),
            filename: format!("Skin {i}.wsz"),
            download_url: format!("https://d/{i}.wsz"),
            screenshot_url: format!("https://s/{i}.png"),
        }
    }

    fn page(from: usize, n: usize, more: bool) -> Page {
        Page {
            skins: (from..from + n).map(skin).collect(),
            sent: if more { crate::museum::PAGE } else { n },
        }
    }

    /// The dialog, with the first page of the museum's list in.
    fn open() -> (Museum, Instant) {
        let now = Instant::now();
        let mut m = Museum::new(1.0, &Fixed, Listings::default(), now);
        let first = m.next_request(now);
        assert_eq!(
            first,
            Some(Request::Page {
                listing: Listing::Museum,
                offset: 0
            })
        );
        assert_eq!(m.next_request(now), None, "once");
        assert!(m.page_arrived(&Listing::Museum, 0, Ok(page(0, 50, true))));
        (m, now)
    }

    fn after(now: Instant, ms: u64) -> Instant {
        now + Duration::from_millis(ms)
    }

    #[test]
    fn it_opens_on_the_museum_list_with_nothing_highlighted_or_tried() {
        let (mut m, now) = open();
        assert_eq!(m.skins().len(), 50);
        assert_eq!(m.highlighted(), None);
        assert_eq!(
            m.next_request(after(now, 5000)),
            None,
            "nothing to fetch unasked"
        );
        assert_eq!(m.status(), "50+ skins");
    }

    #[test]
    fn a_highlighted_skin_shows_its_screenshot_then_is_tried_on() {
        let (mut m, now) = open();
        let _ = m.key(Key::Down, now);
        assert_eq!(m.highlighted(), Some(&skin(0)));
        assert_eq!(m.next_request(after(now, 100)), None, "not yet");
        assert_eq!(
            m.next_request(after(now, 200)),
            Some(Request::Screenshot(skin(0)))
        );
        assert_eq!(m.next_request(after(now, 300)), None);
        assert!(m.screenshot_arrived(&skin(0).md5, Ok(Image::new(275, 348))));
        assert!(m.picture.is_some());
        assert_eq!(
            m.next_request(after(now, 600)),
            Some(Request::Skin(skin(0)))
        );
        assert_eq!(m.caption(), "Fetching Skin 0...");
        assert_eq!(m.next_request(after(now, 700)), None, "once");
        assert_eq!(m.skin_arrived(&skin(0).md5, Ok(())), Arrival::Wear);
        assert_eq!(m.caption(), "Trying on Skin 0");
        assert_eq!(m.key(Key::Enter, now), Outcome::Install);
        assert_eq!(m.caption(), "Installed Skin 0");
        assert_eq!(m.key(Key::Enter, now), Outcome::None, "kept already");
    }

    #[test]
    fn a_skin_arriving_after_the_highlight_moved_on_is_dropped() {
        let (mut m, now) = open();
        let _ = m.key(Key::Down, now);
        assert_eq!(
            m.next_request(after(now, 200)),
            Some(Request::Screenshot(skin(0)))
        );
        assert_eq!(
            m.next_request(after(now, 600)),
            Some(Request::Skin(skin(0)))
        );
        let later = after(now, 700);
        let _ = m.key(Key::Down, later);
        assert_eq!(m.skin_arrived(&skin(0).md5, Ok(())), Arrival::Drop);
        assert_eq!(
            m.next_request(after(later, 200)),
            Some(Request::Screenshot(skin(1)))
        );
        assert_eq!(
            m.next_request(after(later, 600)),
            Some(Request::Skin(skin(1)))
        );
        assert_eq!(
            m.skin_arrived(&skin(1).md5, Err("broken".into())),
            Arrival::Drop
        );
        assert_eq!(m.caption(), "Can't use Skin 1");
        assert_eq!(
            m.next_request(after(later, 5000)),
            None,
            "not again unasked"
        );
        // Install tries again, and keeps it once it arrives.
        assert_eq!(m.key(Key::Enter, later), Outcome::Redraw);
        assert_eq!(m.next_request(later), Some(Request::Skin(skin(1))));
        assert_eq!(m.skin_arrived(&skin(1).md5, Ok(())), Arrival::Install);
    }

    #[test]
    fn install_before_the_skin_is_worn_fetches_it_at_once() {
        let (mut m, now) = open();
        let rows = m.list.rows();
        let (x, y) = (rows.x + 5, rows.y + 2 * m.list.row_h() + 2);
        let _ = m.pointer_down(x, y, now, &Fixed);
        let _ = m.pointer_up(x, y);
        assert_eq!(m.highlighted(), Some(&skin(2)));
        let i = m.layout.install;
        let _ = m.pointer_down(i.x + 5, i.y + 5, now, &Fixed);
        assert_eq!(m.pointer_up(i.x + 5, i.y + 5), Outcome::Redraw);
        // The screenshot does not wait either way, but the skin comes first.
        assert_eq!(m.next_request(now), Some(Request::Skin(skin(2))));
        assert_eq!(m.skin_arrived(&skin(2).md5, Ok(())), Arrival::Install);
        let c = m.layout.close;
        let _ = m.pointer_down(c.x + 5, c.y + 5, now, &Fixed);
        assert_eq!(m.pointer_up(c.x + 5, c.y + 5), Outcome::Close);
        assert_eq!(m.key(Key::Escape, now), Outcome::Close);
    }

    #[test]
    fn typing_searches_once_it_pauses_and_enter_searches_at_once() {
        let (mut m, now) = open();
        let _ = m.key(Key::Down, now);
        let _ = m.text("zel", now);
        assert!(!m.tick(after(now, 300)));
        let _ = m.text("da", after(now, 300));
        assert!(!m.tick(after(now, 600)), "the pause starts again");
        assert!(m.tick(after(now, 800)));
        assert_eq!(m.highlighted(), None);
        let search = Listing::Search("zelda".into());
        assert_eq!(
            m.next_request(after(now, 800)),
            Some(Request::Page {
                listing: search.clone(),
                offset: 0
            })
        );
        assert_eq!(m.status(), "Searching...");
        assert!(m.page_arrived(&search, 0, Ok(page(100, 3, false))));
        assert_eq!(m.status(), "3 skins found");
        // Back to the museum's list, which is still to hand.
        for _ in 0..5 {
            let _ = m.key(Key::Backspace, now);
        }
        assert_eq!(m.key(Key::Enter, now), Outcome::Redraw);
        assert_eq!(m.skins().len(), 50);
        assert_eq!(m.next_request(now), None);
        let _ = m.text("nothing", now);
        let _ = m.key(Key::Enter, now);
        let nothing = Listing::Search("nothing".into());
        assert!(matches!(m.next_request(now), Some(Request::Page { .. })));
        assert!(m.page_arrived(&nothing, 0, Ok(Page::default())));
        assert_eq!(m.status(), "No skins found");
    }

    #[test]
    fn more_is_fetched_near_the_end_and_failures_are_said() {
        let (mut m, now) = open();
        let _ = m.key(Key::PageDown, now);
        let _ = m.key(Key::PageDown, now);
        assert_eq!(m.next_request(now), None, "far from the end");
        for _ in 0..30 {
            let _ = m.key(Key::Down, now);
        }
        let more = Some(Request::Page {
            listing: Listing::Museum,
            offset: 50,
        });
        assert_eq!(m.next_request(now), more);
        assert!(m.page_arrived(&Listing::Museum, 50, Err("offline".into())));
        assert_eq!(m.status(), "50+ skins; can't fetch more");
        assert!(!matches!(m.next_request(now), Some(Request::Page { .. })));
        // A dialog opened again tries again, and finds the list as it was.
        let mut again = Museum::new(1.0, &Fixed, m.take_listings(), now);
        assert!(m.skins().is_empty());
        assert_eq!(again.skins().len(), 50);
        let _ = again.key(Key::PageDown, now);
        for _ in 0..50 {
            let _ = again.key(Key::Down, now);
        }
        assert_eq!(again.next_request(now), more);
        // Pages that arrive late, or twice, are ignored.
        assert!(!again.page_arrived(&Listing::Museum, 0, Ok(page(0, 50, true))));
        assert!(again.page_arrived(&Listing::Museum, 50, Ok(page(50, 20, false))));
        assert_eq!(again.status(), "70 skins");
    }

    #[test]
    fn an_empty_museum_that_cannot_be_reached_says_so() {
        let now = Instant::now();
        let mut m = Museum::new(1.0, &Fixed, Listings::default(), now);
        let _ = m.next_request(now);
        assert_eq!(m.status(), "Loading...");
        assert!(m.page_arrived(&Listing::Museum, 0, Err("no network".into())));
        assert_eq!(m.status(), "Can't reach the museum");
        assert_eq!(m.key(Key::Down, now), Outcome::Redraw);
        assert_eq!(m.highlighted(), None);
        assert_eq!(m.key(Key::Enter, now), Outcome::None);
    }

    #[test]
    fn choosing_another_skin_stops_trying_until_the_highlight_moves() {
        let (mut m, now) = open();
        let _ = m.key(Key::Down, now);
        let _ = m.next_request(after(now, 200));
        assert_eq!(
            m.next_request(after(now, 600)),
            Some(Request::Skin(skin(0)))
        );
        assert_eq!(m.skin_arrived(&skin(0).md5, Ok(())), Arrival::Wear);
        m.dropped();
        assert_eq!(m.caption(), "Skin 0");
        assert_eq!(m.next_request(after(now, 5000)), None);
        let _ = m.key(Key::Down, after(now, 5000));
        assert_eq!(
            m.next_request(after(now, 5600)),
            Some(Request::Screenshot(skin(1)))
        );
    }

    #[test]
    fn it_draws_the_screenshot_at_the_dialogs_scale() {
        let font = TtfFont::load_menu(crate::menu::font_px(2.0));
        let now = Instant::now();
        let mut m = Museum::new(2.0, &font, Listings::default(), now);
        let _ = m.next_request(now);
        m.page_arrived(&Listing::Museum, 0, Ok(page(0, 5, false)));
        let _ = m.key(Key::Down, now);
        let mut shot = Image::new(275, 348);
        shot.put(1, 1, [255, 0, 0, 255]);
        m.screenshot_arrived(&skin(0).md5, Ok(shot));
        let (w, h) = m.size();
        assert_eq!((w, h), (1200, 840));
        let mut c = Image::new(w, h);
        m.draw(&mut c, &font, now);
        let p = m.layout.picture;
        assert_eq!((p.w, p.h), (550, 696));
        assert_eq!(
            c.get(p.x + 3, p.y + 3),
            [255, 0, 0, 255],
            "pixel (1, 1), doubled"
        );
        assert_eq!(c.get(p.x + 1, p.y + 1), [0, 0, 0, 0]);
        let rows = m.list.rows();
        assert_eq!(c.get(rows.right() - 2, rows.y + 2), classic::SELECTION);
        assert!(m.layout.shot.right() <= w && m.layout.caption.bottom() <= m.layout.install.y);
    }
}
