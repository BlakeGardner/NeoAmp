//! The File info dialog (the clutter bar's I, Alt+3, the playlist's MISC > FILE INFO): a
//! file's tags and what it holds, drawn like the Windows 2000 dialogs Winamp 2.x showed.
//! Winamp's could edit the tags; NeoAmp's only shows them, in read-only boxes. For an internet
//! stream it shows what the station says about itself, and the song playing.
//!
//! This is the dialog's state, layout, input and drawing; `info_window` gives it an OS window
//! and reads the file. Geometry is in physical pixels, as in [`crate::jump`].

use crate::audio::{FileDetails, Location, Station, TrackInfo};
use crate::canvas::{Image, Rect};
use crate::classic::{self, FACE, TEXT, line, px};
use crate::font::TtfFont;
use crate::menu::Measure;
use crate::ui::{push_fmt, write_time};

/// The window's title.
pub const TITLE: &str = "File info";

/// Sizes at 96 DPI, which `px` scales.
mod metric {
    pub const W: i32 = 500;
    pub const MARGIN: i32 = 8;
    pub const GAP: i32 = 6;
    pub const EDIT_H: i32 = 20;
    /// Between two rows of edit boxes.
    pub const ROW_GAP: i32 = 4;
    /// From an edit box's edge to its text.
    pub const TEXT_PAD: i32 = 4;
    /// The Tags group's width; Details has the rest.
    pub const TAGS_W: i32 = 270;
    /// A group box's caption, which its top edge runs through.
    pub const CAPTION_H: i32 = 14;
    /// From a group box's frame to what it holds.
    pub const GROUP_PAD: i32 = 8;
    /// A line of the details.
    pub const LINE_H: i32 = 15;
    /// How far a detail that runs onto another line is indented there.
    pub const HANGING: i32 = 12;
    pub const BUTTON_W: i32 = 75;
    pub const BUTTON_H: i32 = 23;
}

const FILE: &str = "File:";
const TAGS: &str = "Tags";
const DETAILS: &str = "Details";
const CLOSE: &str = "Close";
const ELLIPSIS: char = '…';

/// The tags, by row: label and which tag.
const TAG_ROWS: [(&str, Tag); 6] = [
    ("Title:", Tag::Title),
    ("Artist:", Tag::Artist),
    ("Album:", Tag::Album),
    ("Year:", Tag::Year),
    ("Genre:", Tag::Genre),
    ("Comment:", Tag::Comment),
];
/// Shares the year's row.
const TRACK: &str = "Track:";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tag {
    Title,
    Artist,
    Album,
    Year,
    Genre,
    Comment,
}

/// What a key does in the dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// Enter, Escape and Space all close it: Close is its only button.
    Enter,
    Escape,
    Space,
}

/// What the window layer must do after an input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    None,
    Redraw,
    Close,
}

/// What is known about the file.
#[derive(Clone, Debug, PartialEq)]
pub enum Contents {
    /// It is still being read.
    Reading,
    Read(Box<FileDetails>),
    /// An internet stream.
    Stream(Box<StreamDetails>),
    /// It could not be read, for this reason.
    Failed(String),
}

/// What is known about an internet stream: only what it said while it played, if it is the
/// one playing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreamDetails {
    /// `None` unless the stream is playing and has been recognised.
    pub station: Option<Station>,
    /// The song playing now, if the station names it.
    pub song: Option<String>,
    /// The audio's sample rate, channels and bitrate, once it plays.
    pub info: Option<TrackInfo>,
}

/// Where everything is, in the dialog's pixels.
#[derive(Clone, Copy, Debug)]
struct Layout {
    size: (i32, i32),
    /// One-pixel lines at this scale.
    t: i32,
    file_label_x: i32,
    file: Rect,
    tags: Rect,
    details: Rect,
    /// Where the tags' labels start, and their boxes.
    label_x: i32,
    /// One box per row of [`TAG_ROWS`]; the year's is the left part of its row.
    boxes: [Rect; 6],
    track_label_x: i32,
    track: Rect,
    /// The first line of the details.
    details_text: Rect,
    line_h: i32,
    close: Rect,
}

impl Layout {
    fn new(scale: f64, m: &dyn Measure) -> Layout {
        let p = |v| px(v, scale);
        let t = line(scale);
        let w = p(metric::W);
        let margin = p(metric::MARGIN);
        let gap = p(metric::GAP);
        let edit_h = p(metric::EDIT_H);
        let pad = p(metric::GROUP_PAD);

        let file_x = margin + m.text_width(FILE) + gap;
        let file = Rect::new(file_x, margin, w - margin - file_x, edit_h);

        let groups_y = file.bottom() + gap;
        let content_y = groups_y + p(metric::CAPTION_H) + p(metric::ROW_GAP);
        let row_step = edit_h + p(metric::ROW_GAP);
        let rows = i32::try_from(TAG_ROWS.len()).unwrap_or(0);
        let groups_h = content_y - groups_y + rows * row_step - p(metric::ROW_GAP) + pad;
        let tags = Rect::new(margin, groups_y, p(metric::TAGS_W), groups_h);
        let details_x = tags.right() + gap;
        let details = Rect::new(details_x, groups_y, w - margin - details_x, groups_h);

        let label_x = tags.x + pad;
        let label_w = TAG_ROWS
            .iter()
            .map(|(label, _)| m.text_width(label))
            .max()
            .unwrap_or(0);
        let box_x = label_x + label_w + gap;
        let box_w = tags.right() - pad - box_x;
        let mut boxes: [Rect; 6] = std::array::from_fn(|i| {
            let y = content_y + i32::try_from(i).unwrap_or(0) * row_step;
            Rect::new(box_x, y, box_w, edit_h)
        });
        // The year and the track share a row, half the room each.
        let year_row = TAG_ROWS
            .iter()
            .position(|(_, tag)| *tag == Tag::Year)
            .unwrap_or(0);
        let track_label_w = m.text_width(TRACK);
        let half = (box_w - track_label_w - 2 * gap) / 2;
        let year = Rect::new(box_x, boxes[year_row].y, half, edit_h);
        let track_label_x = year.right() + gap;
        let track_x = track_label_x + track_label_w + gap;
        let track = Rect::new(track_x, year.y, tags.right() - pad - track_x, edit_h);
        boxes[year_row] = year;

        let details_text = Rect::new(
            details.x + pad,
            content_y,
            details.w - 2 * pad,
            details.bottom() - pad - content_y,
        );

        let button = (p(metric::BUTTON_W), p(metric::BUTTON_H));
        let close = Rect::new(
            w - margin - button.0,
            tags.bottom() + gap + p(2),
            button.0,
            button.1,
        );
        Layout {
            size: (w, close.bottom() + margin),
            t,
            file_label_x: margin,
            file,
            tags,
            details,
            label_x,
            boxes,
            track_label_x,
            track,
            details_text,
            line_h: p(metric::LINE_H).max(1),
            close,
        }
    }
}

/// The dialog.
#[derive(Debug)]
pub struct Info {
    scale: f64,
    layout: Layout,
    location: Location,
    contents: Contents,
    /// Close is held down.
    pressed: bool,
    /// The last pointer position: the held button shows pushed in only while under it.
    pointer: (i32, i32),
}

impl Info {
    /// A dialog about a file or stream, still being read, at `scale` times Windows' 96 DPI
    /// sizes, measuring text with `m`.
    #[must_use]
    pub fn new(scale: f64, m: &dyn Measure, location: Location) -> Info {
        Info {
            scale,
            layout: Layout::new(scale, m),
            location,
            contents: Contents::Reading,
            pressed: false,
            pointer: (-1, -1),
        }
    }

    /// The window's size in pixels.
    #[must_use]
    pub fn size(&self) -> (i32, i32) {
        self.layout.size
    }

    /// Lay the dialog out again at another scale.
    pub fn rescale(&mut self, scale: f64, m: &dyn Measure) {
        self.scale = scale;
        self.layout = Layout::new(scale, m);
    }

    #[must_use]
    pub fn location(&self) -> &Location {
        &self.location
    }

    #[must_use]
    pub fn contents(&self) -> &Contents {
        &self.contents
    }

    /// Show another file or stream, still to be read.
    pub fn show(&mut self, location: Location) {
        self.location = location;
        self.contents = Contents::Reading;
    }

    /// The file has been read, or could not be.
    pub fn set_contents(&mut self, contents: Contents) {
        self.contents = contents;
    }

    /// A key pressed.
    pub fn key(&mut self, key: Key) -> Outcome {
        match key {
            Key::Enter | Key::Escape | Key::Space => Outcome::Close,
        }
    }

    fn pushed(&self) -> bool {
        self.pressed && self.layout.close.contains(self.pointer.0, self.pointer.1)
    }

    /// The pointer pressed at (`x`, `y`).
    pub fn pointer_down(&mut self, x: i32, y: i32) -> Outcome {
        self.pointer = (x, y);
        if self.layout.close.contains(x, y) {
            self.pressed = true;
            Outcome::Redraw
        } else {
            Outcome::None
        }
    }

    /// The pointer moved to (`x`, `y`): a held button pops out while the pointer is off it.
    pub fn pointer_moved(&mut self, x: i32, y: i32) -> Outcome {
        let was = self.pushed();
        self.pointer = (x, y);
        if self.pushed() == was {
            Outcome::None
        } else {
            Outcome::Redraw
        }
    }

    /// The pointer let go at (`x`, `y`): Close pressed and let go over closes.
    pub fn pointer_up(&mut self, x: i32, y: i32) -> Outcome {
        self.pointer = (x, y);
        if !std::mem::take(&mut self.pressed) {
            return Outcome::None;
        }
        if self.layout.close.contains(x, y) {
            Outcome::Close
        } else {
            Outcome::Redraw
        }
    }

    fn tag(&self, tag: Tag) -> Option<&str> {
        let d = match &self.contents {
            Contents::Read(d) => d,
            Contents::Stream(s) => return stream_tag(s, tag),
            Contents::Reading | Contents::Failed(_) => return None,
        };
        let t = &d.tags;
        match tag {
            Tag::Title => t.title.as_deref(),
            Tag::Artist => t.artist.as_deref(),
            Tag::Album => t.album.as_deref(),
            Tag::Year => t.year.as_deref(),
            Tag::Genre => t.genre.as_deref(),
            Tag::Comment => t.comment.as_deref(),
        }
    }

    fn track(&self) -> Option<&str> {
        match &self.contents {
            Contents::Read(d) => d.tags.track.as_deref(),
            _ => None,
        }
    }

    /// Draw the dialog into `c`, which must be [`Info::size`]. `font` must be the menu font at
    /// this scale ([`crate::menu::font_px`]).
    pub fn draw(&self, c: &mut Image, font: &TtfFont) {
        let l = &self.layout;
        let t = l.t;
        let lh = font.line_height();
        c.clear(FACE);
        let all = c.bounds();
        let centred = |r: Rect| r.y + (r.h - lh) / 2;

        font.draw(c, FILE, l.file_label_x, centred(l.file), TEXT, all);
        let room = l.file.w - 2 * px(metric::TEXT_PAD, self.scale);
        let path = self.location.to_string();
        self.read_only_box(c, font, l.file, &fit_end(&path, room, font));

        classic::group_box(c, font, l.tags, t, self.scale, TAGS);
        for ((label, tag), r) in TAG_ROWS.iter().zip(l.boxes) {
            font.draw(c, label, l.label_x, centred(r), TEXT, all);
            self.read_only_box(c, font, r, self.tag(*tag).unwrap_or(""));
        }
        font.draw(c, TRACK, l.track_label_x, centred(l.track), TEXT, all);
        self.read_only_box(c, font, l.track, self.track().unwrap_or(""));

        classic::group_box(c, font, l.details, t, self.scale, DETAILS);
        let text = l.details_text;
        let hanging = px(metric::HANGING, self.scale);
        let mut y = text.y;
        for detail in detail_lines(&self.contents) {
            for (i, part) in wrap(&detail, text.w, hanging, font).iter().enumerate() {
                let x = if i == 0 { text.x } else { text.x + hanging };
                font.draw(c, part, x, y, TEXT, text);
                y += l.line_h;
            }
        }

        classic::push_button(c, font, l.close, t, CLOSE, true, self.pushed());
    }

    /// A read-only edit box holding `text`: sunken, on the dialog's grey, as Windows draws
    /// them.
    fn read_only_box(&self, c: &mut Image, font: &TtfFont, r: Rect, text: &str) {
        let t = self.layout.t;
        classic::sunken(c, r, t);
        let inner = Rect::new(r.x + 2 * t, r.y + 2 * t, r.w - 4 * t, r.h - 4 * t);
        let x = r.x + px(metric::TEXT_PAD, self.scale);
        let y = r.y + (r.h - font.line_height()) / 2;
        font.draw(c, text, x, y, TEXT, inner);
    }
}

/// A stream's "tags": the song playing as the title, and what the station says of itself.
fn stream_tag(s: &StreamDetails, tag: Tag) -> Option<&str> {
    let station = s.station.as_ref();
    match tag {
        Tag::Title => s.song.as_deref(),
        Tag::Genre => station?.genre.as_deref(),
        Tag::Comment => station?.description.as_deref(),
        Tag::Artist | Tag::Album | Tag::Year => None,
    }
}

/// `text` if it fits in `width`, else as much of its end as fits after an ellipsis: a path
/// keeps its file name.
fn fit_end(text: &str, width: i32, m: &dyn Measure) -> String {
    if m.text_width(text) <= width {
        return text.to_owned();
    }
    let mut start = text.len();
    for (i, _) in text.char_indices().rev() {
        let candidate = format!("{ELLIPSIS}{}", &text[i..]);
        if m.text_width(&candidate) > width {
            break;
        }
        start = i;
    }
    format!("{ELLIPSIS}{}", &text[start..])
}

/// Break `text` into lines `width` wide at spaces, the lines after the first `indent` narrower.
/// A word too long for a line is broken between characters.
fn wrap(text: &str, width: i32, indent: i32, m: &dyn Measure) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let room = |lines: &Vec<String>| {
        if lines.is_empty() {
            width
        } else {
            width - indent
        }
    };
    for word in text.split(' ') {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if m.text_width(&candidate) <= room(&lines) {
            current = candidate;
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        // The word starts a line; break it up if even that is too narrow.
        for ch in word.chars() {
            current.push(ch);
            if m.text_width(&current) > room(&lines) && current.chars().count() > 1 {
                current.pop();
                lines.push(std::mem::take(&mut current));
                current.push(ch);
            }
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// `n` with its thousands grouped by commas, as Winamp showed sizes: "4,123,456".
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The lines of the Details group.
fn detail_lines(contents: &Contents) -> Vec<String> {
    let d = match contents {
        Contents::Reading => return vec!["Reading the file…".to_owned()],
        Contents::Failed(why) => return vec!["Cannot read this file:".to_owned(), why.clone()],
        Contents::Stream(s) => return stream_lines(s),
        Contents::Read(d) => d,
    };
    let mut lines = Vec::with_capacity(8);
    let mut add = |args: std::fmt::Arguments<'_>| {
        let mut s = String::new();
        push_fmt(&mut s, args);
        lines.push(s);
    };
    if !d.format.is_empty() {
        add(format_args!("Format: {}", d.format));
    }
    if !d.codec.is_empty() {
        add(format_args!("Codec: {}", d.codec));
    }
    match d.info.seconds {
        Some(secs) => {
            // Rounded, non-negative and far below u32::MAX, so the cast cannot truncate.
            let whole = secs.round().max(0.0) as u32;
            let mut time = String::new();
            write_time(&mut time, whole);
            add(format_args!(
                "Length: {time} ({} seconds)",
                grouped(whole.into())
            ));
        }
        None => add(format_args!("Length: unknown")),
    }
    if let Some(kbps) = d.info.kbps {
        add(format_args!("Average bitrate: {kbps} kbit/s"));
    }
    add(format_args!(
        "Sample rate: {} Hz",
        grouped(d.info.sample_rate.into())
    ));
    match d.info.channels {
        1 => add(format_args!("Channels: 1 (mono)")),
        2 => add(format_args!("Channels: 2 (stereo)")),
        n => add(format_args!("Channels: {n}")),
    }
    if let Some(bits) = d.bits_per_sample {
        add(format_args!("Bits per sample: {bits}"));
    }
    if let Some(size) = d.size {
        add(format_args!("File size: {} bytes", grouped(size)));
    }
    lines
}

/// The Details group's lines for a stream.
fn stream_lines(s: &StreamDetails) -> Vec<String> {
    let mut lines = vec!["Internet radio stream".to_owned()];
    let Some(station) = &s.station else {
        lines.push("More shows here while it plays.".to_owned());
        return lines;
    };
    let mut add = |args: std::fmt::Arguments<'_>| {
        let mut s = String::new();
        push_fmt(&mut s, args);
        lines.push(s);
    };
    if let Some(name) = &station.name {
        add(format_args!("Station: {name}"));
    }
    if let Some(homepage) = &station.homepage {
        add(format_args!("Web site: {homepage}"));
    }
    if !station.format.is_empty() {
        add(format_args!("Format: {}", station.format));
    }
    if !station.codec.is_empty() {
        add(format_args!("Codec: {}", station.codec));
    }
    if let Some(kbps) = station.kbps.or_else(|| s.info.as_ref()?.kbps) {
        add(format_args!("Bitrate: {kbps} kbit/s"));
    }
    if let Some(info) = &s.info {
        add(format_args!(
            "Sample rate: {} Hz",
            grouped(info.sample_rate.into())
        ));
        match info.channels {
            1 => add(format_args!("Channels: 1 (mono)")),
            2 => add(format_args!("Channels: 2 (stereo)")),
            n => add(format_args!("Channels: {n}")),
        }
    }
    lines
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::audio::Tags;

    /// Every character 6 pixels wide.
    struct Fixed;

    impl Measure for Fixed {
        fn text_width(&self, text: &str) -> i32 {
            i32::try_from(text.chars().count()).unwrap() * 6
        }
    }

    fn details() -> FileDetails {
        FileDetails {
            tags: Tags {
                title: Some("Rolling Llama".into()),
                artist: Some("Justin F.".into()),
                track: Some("3/9".into()),
                ..Tags::default()
            },
            format: "MPEG Audio Layer 3 Native".into(),
            codec: "MPEG Audio Layer 3".into(),
            info: TrackInfo {
                title: Some("Justin F. - Rolling Llama".into()),
                seconds: Some(254.4),
                sample_rate: 44_100,
                channels: 2,
                kbps: Some(192),
            },
            bits_per_sample: None,
            size: Some(6_123_456),
        }
    }

    #[test]
    fn the_details_say_what_the_file_holds() {
        let lines = detail_lines(&Contents::Read(Box::new(details())));
        assert_eq!(
            lines,
            [
                "Format: MPEG Audio Layer 3 Native",
                "Codec: MPEG Audio Layer 3",
                "Length: 4:14 (254 seconds)",
                "Average bitrate: 192 kbit/s",
                "Sample rate: 44,100 Hz",
                "Channels: 2 (stereo)",
                "File size: 6,123,456 bytes",
            ]
        );
        let mut mono = details();
        mono.info.channels = 1;
        mono.info.seconds = None;
        mono.bits_per_sample = Some(16);
        let lines = detail_lines(&Contents::Read(Box::new(mono)));
        assert!(lines.contains(&"Channels: 1 (mono)".to_owned()));
        assert!(lines.contains(&"Length: unknown".to_owned()));
        assert!(lines.contains(&"Bits per sample: 16".to_owned()));
        assert_eq!(detail_lines(&Contents::Reading).len(), 1);
        assert_eq!(
            detail_lines(&Contents::Failed("no audio track".into()))[1],
            "no audio track"
        );
    }

    #[test]
    fn streams_show_their_station_and_song() {
        let not_playing = StreamDetails::default();
        assert_eq!(
            detail_lines(&Contents::Stream(Box::new(not_playing))),
            ["Internet radio stream", "More shows here while it plays."]
        );
        let playing = StreamDetails {
            station: Some(Station {
                name: Some("Radio Llama".into()),
                genre: Some("Ambient".into()),
                homepage: Some("https://radio.example".into()),
                description: Some("Whipping llamas since 1997".into()),
                kbps: Some(128),
                format: "MPEG Audio".into(),
                codec: "MPEG Audio Layer 3".into(),
            }),
            song: Some("Justin F. - Rolling Llama".into()),
            info: Some(TrackInfo {
                sample_rate: 44_100,
                channels: 2,
                ..TrackInfo::default()
            }),
        };
        let mut info = Info::new(
            1.0,
            &Fixed,
            Location::Url("http://radio.example/live".into()),
        );
        info.set_contents(Contents::Stream(Box::new(playing)));
        assert_eq!(
            detail_lines(info.contents()),
            [
                "Internet radio stream",
                "Station: Radio Llama",
                "Web site: https://radio.example",
                "Format: MPEG Audio",
                "Codec: MPEG Audio Layer 3",
                "Bitrate: 128 kbit/s",
                "Sample rate: 44,100 Hz",
                "Channels: 2 (stereo)",
            ]
        );
        assert_eq!(info.tag(Tag::Title), Some("Justin F. - Rolling Llama"));
        assert_eq!(info.tag(Tag::Genre), Some("Ambient"));
        assert_eq!(info.tag(Tag::Comment), Some("Whipping llamas since 1997"));
        assert_eq!(info.tag(Tag::Artist), None);
        assert_eq!(info.track(), None);
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(44_100), "44,100");
        assert_eq!(grouped(1_234_567_890), "1,234,567,890");
    }

    #[test]
    fn long_paths_keep_their_end() {
        assert_eq!(fit_end("/a/b.mp3", 60, &Fixed), "/a/b.mp3");
        // Ten characters fit: the ellipsis and the last nine.
        assert_eq!(fit_end("/music/album/song.mp3", 60, &Fixed), "…/song.mp3");
        assert_eq!(fit_end("ünïcödé", 24, &Fixed), "…ödé");
    }

    #[test]
    fn details_wrap_at_spaces_with_a_hanging_indent() {
        let lines = wrap("Codec: PCM Signed 16-bit Big-Endian", 96, 12, &Fixed);
        assert_eq!(lines, ["Codec: PCM", "Signed 16-bit", "Big-Endian"]);
        // A word wider than a line is broken up.
        let lines = wrap("abcdefghij", 36, 0, &Fixed);
        assert_eq!(lines, ["abcdef", "ghij"]);
        assert_eq!(wrap("", 36, 0, &Fixed), [""]);
    }

    #[test]
    fn close_closes_from_the_button_and_the_keys() {
        let mut info = Info::new(1.0, &Fixed, Location::File("/m/a.mp3".into()));
        let r = info.layout.close;
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        assert_eq!(info.pointer_down(x, y), Outcome::Redraw);
        assert!(info.pushed());
        assert_eq!(info.pointer_moved(0, 0), Outcome::Redraw);
        assert!(!info.pushed(), "pops out off the button");
        assert_eq!(info.pointer_up(0, 0), Outcome::Redraw, "let go elsewhere");
        let _ = info.pointer_down(x, y);
        assert_eq!(info.pointer_up(x, y), Outcome::Close);
        assert_eq!(info.pointer_down(1, 1), Outcome::None);
        assert_eq!(info.key(Key::Escape), Outcome::Close);
        assert_eq!(info.key(Key::Enter), Outcome::Close);
    }

    #[test]
    fn the_layout_fits_in_the_window() {
        let info = Info::new(1.5, &Fixed, Location::File("/m/a.mp3".into()));
        let l = info.layout;
        let window = Rect::new(0, 0, l.size.0, l.size.1);
        for r in [l.file, l.tags, l.details, l.track, l.close]
            .into_iter()
            .chain(l.boxes)
        {
            assert_eq!(window.intersect(&r), r, "{r:?}");
        }
        assert!(l.boxes.iter().all(|b| l.tags.intersect(b) == *b));
        assert!(
            l.track.x > l.boxes[3].right(),
            "the track is right of the year"
        );
        assert!(l.close.y > l.details.bottom());
    }

    #[test]
    fn it_draws_its_tags() {
        let font = TtfFont::load_menu(crate::menu::font_px(1.0));
        let mut info = Info::new(1.0, &font, Location::File("/m/a.mp3".into()));
        info.set_contents(Contents::Read(Box::new(details())));
        let (w, h) = info.size();
        let mut c = Image::new(w, h);
        info.draw(&mut c, &font);
        let l = info.layout;
        assert_eq!(c.get(l.file.x + 10, l.file.y), classic::SHADOW);
        assert_eq!(c.get(l.close.x, l.close.y), classic::FRAME);
        // The title's box holds dark text; the album's, with no album, none.
        let has_text = |r: Rect| {
            (r.y + 3..r.bottom() - 3)
                .any(|y| (r.x + 3..r.right() - 3).any(|x| c.get(x, y)[0] < 0x60))
        };
        assert!(has_text(l.boxes[0]));
        assert!(!has_text(l.boxes[2]));
        assert!(has_text(l.track));
        assert_eq!(info.tag(Tag::Artist), Some("Justin F."));
    }
}
