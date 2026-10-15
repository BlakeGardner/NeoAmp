//! A skin's mouse pointers: the `.cur` files Winamp 2.x showed over each part of its windows
//! (`TITLEBAR.CUR` over the main title bar, `PSIZE.CUR` over the playlist's grip, ...).
//!
//! A `.cur` file is a Windows cursor: an icon file whose directory gives each image's hotspot.
//! Some skins name animated cursors (`.ani`, a RIFF file of frames) `.cur`, as Winamp allowed;
//! their first frame is shown, still.

use anyhow::{Context, Result, bail};

/// Largest cursor side we accept from an untrusted archive.
const MAX_SIDE: u32 = 256;

/// Each pointer a skin may have, named after its file. Which one shows where is
/// [`crate::ui::Ui::cursor_at`]'s concern. Winamp also read `VOLBAR`, `WSCLOSE`, `WSMIN` and
/// `WSWINBUT`, but its later versions never showed them, and NeoAmp does not either.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SkinCursor {
    /// The main window's close button.
    Close,
    EqClose,
    /// The equalizer, where nothing else applies.
    EqNormal,
    /// The equalizer's sliders.
    EqSlid,
    /// The equalizer's title bar, and the whole of its windowshade bar.
    EqTitle,
    /// The main window's options button.
    MainMenu,
    /// The options button of the main window's windowshade bar.
    MMenu,
    /// The main window's minimize button.
    Min,
    /// The main window, where nothing else applies.
    Normal,
    PClose,
    /// The playlist, where nothing else applies.
    PNormal,
    /// The position bar.
    PosBar,
    /// The playlist's resize grip.
    PSize,
    /// The playlist's title bar.
    PTBar,
    /// The playlist's scroll bar.
    PVScroll,
    /// The playlist's windowshade button.
    PWinBut,
    /// The playlist's windowshade bar, where nothing else applies.
    PWsNorm,
    /// The resize grip of the playlist's windowshade bar.
    PWsSize,
    /// The song title in the main window.
    SongName,
    /// The main window's title bar.
    TitleBar,
    /// The volume and balance sliders.
    VolBal,
    /// The main window's windowshade button.
    WinBut,
    /// The main window's windowshade bar, where nothing else applies.
    WsNormal,
    /// The position bar of the main window's windowshade bar.
    WsPosBar,
}

impl SkinCursor {
    pub const ALL: [SkinCursor; 24] = [
        SkinCursor::Close,
        SkinCursor::EqClose,
        SkinCursor::EqNormal,
        SkinCursor::EqSlid,
        SkinCursor::EqTitle,
        SkinCursor::MainMenu,
        SkinCursor::MMenu,
        SkinCursor::Min,
        SkinCursor::Normal,
        SkinCursor::PClose,
        SkinCursor::PNormal,
        SkinCursor::PosBar,
        SkinCursor::PSize,
        SkinCursor::PTBar,
        SkinCursor::PVScroll,
        SkinCursor::PWinBut,
        SkinCursor::PWsNorm,
        SkinCursor::PWsSize,
        SkinCursor::SongName,
        SkinCursor::TitleBar,
        SkinCursor::VolBal,
        SkinCursor::WinBut,
        SkinCursor::WsNormal,
        SkinCursor::WsPosBar,
    ];

    /// The file it comes from, lowercase.
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            SkinCursor::Close => "close.cur",
            SkinCursor::EqClose => "eqclose.cur",
            SkinCursor::EqNormal => "eqnormal.cur",
            SkinCursor::EqSlid => "eqslid.cur",
            SkinCursor::EqTitle => "eqtitle.cur",
            SkinCursor::MainMenu => "mainmenu.cur",
            SkinCursor::MMenu => "mmenu.cur",
            SkinCursor::Min => "min.cur",
            SkinCursor::Normal => "normal.cur",
            SkinCursor::PClose => "pclose.cur",
            SkinCursor::PNormal => "pnormal.cur",
            SkinCursor::PosBar => "posbar.cur",
            SkinCursor::PSize => "psize.cur",
            SkinCursor::PTBar => "ptbar.cur",
            SkinCursor::PVScroll => "pvscroll.cur",
            SkinCursor::PWinBut => "pwinbut.cur",
            SkinCursor::PWsNorm => "pwsnorm.cur",
            SkinCursor::PWsSize => "pwssize.cur",
            SkinCursor::SongName => "songname.cur",
            SkinCursor::TitleBar => "titlebar.cur",
            SkinCursor::VolBal => "volbal.cur",
            SkinCursor::WinBut => "winbut.cur",
            SkinCursor::WsNormal => "wsnormal.cur",
            SkinCursor::WsPosBar => "wsposbar.cur",
        }
    }
}

/// A decoded cursor: RGBA pixels, transparent where the cursor shows what is under it.
#[derive(Clone, PartialEq, Eq)]
pub struct CursorImage {
    pub w: u16,
    pub h: u16,
    /// The pixel that points, from the top-left corner.
    pub hotspot: (u16, u16),
    /// `w` x `h` RGBA pixels, row by row.
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for CursorImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CursorImage")
            .field("w", &self.w)
            .field("h", &self.h)
            .field("hotspot", &self.hotspot)
            .finish_non_exhaustive()
    }
}

impl CursorImage {
    /// The same cursor `k` times larger, each pixel a `k` x `k` square, for screens whose
    /// desktop is scaled up.
    #[must_use]
    pub fn scaled(&self, k: u16) -> CursorImage {
        let k = k.max(1);
        if k == 1 {
            return self.clone();
        }
        let (w, h) = (usize::from(self.w), usize::from(self.h));
        let k_us = usize::from(k);
        let mut rgba = Vec::with_capacity(self.rgba.len() * k_us * k_us);
        for row in self.rgba.chunks_exact(w * 4).take(h) {
            let mut wide = Vec::with_capacity(row.len() * k_us);
            for px in row.chunks_exact(4) {
                for _ in 0..k {
                    wide.extend_from_slice(px);
                }
            }
            for _ in 0..k {
                rgba.extend_from_slice(&wide);
            }
        }
        CursorImage {
            w: self.w.saturating_mul(k),
            h: self.h.saturating_mul(k),
            hotspot: (
                self.hotspot.0.saturating_mul(k),
                self.hotspot.1.saturating_mul(k),
            ),
            rgba,
        }
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Decode a `.cur` file (or an `.ani` one by that name).
///
/// # Errors
/// Fails if `bytes` is neither, is damaged, or holds no image of a sensible size.
pub fn parse(bytes: &[u8]) -> Result<CursorImage> {
    if bytes.starts_with(b"RIFF") {
        return parse(first_ani_frame(bytes)?);
    }
    let count = u16_at(bytes, 4).context("truncated cursor header")?;
    if u16_at(bytes, 0) != Some(0) || count == 0 {
        bail!("not a cursor file");
    }
    // Each entry: width, height (0 meaning 256), colours, reserved, hotspot x and y, size of
    // the image and where it starts.
    struct Entry {
        w: u32,
        h: u32,
        hotspot: (u16, u16),
        data: (usize, usize),
    }
    let entries: Vec<Entry> = (0..usize::from(count))
        .filter_map(|i| {
            let at = 6 + 16 * i;
            let side = |b: u8| if b == 0 { 256 } else { u32::from(b) };
            let size = usize::try_from(u32_at(bytes, at + 8)?).ok()?;
            let offset = usize::try_from(u32_at(bytes, at + 12)?).ok()?;
            Some(Entry {
                w: side(*bytes.get(at)?),
                h: side(*bytes.get(at + 1)?),
                hotspot: (u16_at(bytes, at + 4)?, u16_at(bytes, at + 6)?),
                data: (offset, size),
            })
        })
        .collect();
    // Winamp's cursors are 32 x 32; with several sizes, that one, else the largest.
    let entry = entries
        .iter()
        .find(|e| (e.w, e.h) == (32, 32))
        .or_else(|| entries.iter().max_by_key(|e| e.w * e.h))
        .context("no cursor images")?;
    let (offset, size) = entry.data;
    let data = bytes
        .get(offset..offset.saturating_add(size))
        .context("cursor image past the end of the file")?;
    // An icon file holding just that image, which the image crate decodes, transparency mask
    // and all. Pixels that invert the screen behind them become transparent.
    let mut ico = Vec::with_capacity(22 + data.len());
    ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    // The sides as the entry gave them: 256 is written 0.
    let side = |v: u32| u8::try_from(v & 0xFF).unwrap_or(0);
    ico.extend_from_slice(&[side(entry.w), side(entry.h), 0, 0]);
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&u32::try_from(data.len())?.to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(data);
    let img = image::load_from_memory_with_format(&ico, image::ImageFormat::Ico)
        .context("decoding the cursor image")?
        .into_rgba8();
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        bail!("unreasonable cursor size {w}x{h}");
    }
    let (w, h) = (u16::try_from(w)?, u16::try_from(h)?);
    Ok(CursorImage {
        w,
        h,
        hotspot: (entry.hotspot.0.min(w - 1), entry.hotspot.1.min(h - 1)),
        rgba: img.into_raw(),
    })
}

/// The first frame of an animated cursor: the first `icon` chunk of its `fram` list.
fn first_ani_frame(bytes: &[u8]) -> Result<&[u8]> {
    if bytes.get(8..12) != Some(b"ACON") {
        bail!("a RIFF file that is not an animated cursor");
    }
    find_icon(bytes.get(12..).unwrap_or_default()).context("an animated cursor with no frames")
}

/// The first `icon` chunk among `chunks`, looking inside `LIST` chunks.
fn find_icon(mut chunks: &[u8]) -> Option<&[u8]> {
    while chunks.len() >= 8 {
        let id = chunks.get(..4)?;
        let size = usize::try_from(u32_at(chunks, 4)?).ok()?;
        let body = chunks.get(8..8usize.checked_add(size)?)?;
        if id == b"icon" {
            return Some(body);
        }
        if id == b"LIST"
            && let Some(icon) = body.get(4..).and_then(find_icon)
        {
            return Some(icon);
        }
        // Chunks are padded to an even length.
        let next = (8 + size).checked_add(size % 2)?;
        chunks = chunks.get(next..)?;
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    fn base_skin_file(name: &str) -> Vec<u8> {
        let mut zip = zip::ZipArchive::new(Cursor::new(crate::app::BASE_SKIN)).unwrap();
        let mut f = zip.by_name(name).unwrap();
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).unwrap();
        bytes
    }

    fn pixel(c: &CursorImage, x: usize, y: usize) -> &[u8] {
        let i = (y * usize::from(c.w) + x) * 4;
        &c.rgba[i..i + 4]
    }

    #[test]
    fn the_base_skins_cursors_decode_with_their_masks() {
        let c = parse(&base_skin_file("TITLEBAR.CUR")).unwrap();
        assert_eq!((c.w, c.h, c.hotspot), (32, 32, (0, 0)));
        assert_eq!(pixel(&c, 0, 0), [0, 0, 0, 255], "the arrow's black tip");
        assert_eq!(pixel(&c, 31, 31)[3], 0, "transparent outside the arrow");
        assert_eq!(pixel(&c, 2, 6)[3], 255, "inside the arrow");
        assert_eq!(c.rgba.len(), 32 * 32 * 4);
    }

    /// A 2 x 2 cursor of 32-bit pixels with its hotspot at (1, 0).
    fn tiny_cur() -> Vec<u8> {
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&4i32.to_le_bytes()); // the image and its mask
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&[0; 24]);
        // Bottom-up BGRA rows, then a mask of 4-byte rows.
        dib.extend_from_slice(&[0, 0, 255, 255, 0, 255, 0, 255]);
        dib.extend_from_slice(&[255, 0, 0, 255, 0, 0, 0, 0]);
        dib.extend_from_slice(&[0; 8]);
        let mut cur = vec![0, 0, 2, 0, 1, 0, 2, 2, 0, 0, 1, 0, 0, 0];
        cur.extend_from_slice(&u32::try_from(dib.len()).unwrap().to_le_bytes());
        cur.extend_from_slice(&22u32.to_le_bytes());
        cur.extend_from_slice(&dib);
        cur
    }

    #[test]
    fn hotspots_and_colours_are_read() {
        let c = parse(&tiny_cur()).unwrap();
        assert_eq!((c.w, c.h, c.hotspot), (2, 2, (1, 0)));
        assert_eq!(pixel(&c, 0, 1), [255, 0, 0, 255], "bottom-up rows");
        assert_eq!(pixel(&c, 0, 0), [0, 0, 255, 255]);
        let big = c.scaled(2);
        assert_eq!((big.w, big.h, big.hotspot), (4, 4, (2, 0)));
        assert_eq!(pixel(&big, 1, 3), [255, 0, 0, 255]);
        assert_eq!(pixel(&big, 1, 0), [0, 0, 255, 255]);
        assert_eq!(pixel(&big, 2, 0)[3], 0);
    }

    #[test]
    fn an_animated_cursor_shows_its_first_frame() {
        let frame = tiny_cur();
        let chunk = |id: &[u8], body: &[u8]| {
            let mut c = id.to_vec();
            c.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
            c.extend_from_slice(body);
            if body.len() % 2 == 1 {
                c.push(0);
            }
            c
        };
        let mut fram = b"fram".to_vec();
        fram.extend(chunk(b"icon", &frame));
        fram.extend(chunk(b"icon", b"second"));
        let mut body = b"ACON".to_vec();
        body.extend(chunk(b"anih", &[0; 36]));
        body.extend(chunk(b"LIST", b"INFOx"));
        body.extend(chunk(b"LIST", &fram));
        let ani = chunk(b"RIFF", &body);
        assert_eq!(parse(&ani).unwrap(), parse(&frame).unwrap());
    }

    #[test]
    fn damaged_files_are_rejected_not_panicked_on() {
        assert!(parse(b"").is_err());
        assert!(parse(b"not a cursor at all").is_err());
        let cur = tiny_cur();
        assert!(parse(&cur[..30]).is_err(), "cut short");
        assert!(parse(b"RIFF\x04\x00\x00\x00ACON").is_err(), "no frames");
        let mut lying = cur.clone();
        lying[14..18].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&lying).is_err(), "an image past the end");
    }

    #[test]
    fn every_cursor_has_its_own_file() {
        let mut names: Vec<&str> = SkinCursor::ALL.iter().map(|c| c.file_name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SkinCursor::ALL.len());
    }
}
