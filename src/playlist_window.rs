//! Drawing of the resizable playlist editor window.

use std::time::Instant;

use crate::canvas::{Image, Rect};
use crate::font::{TtfFont, bitmap_text_width, draw_bitmap_text};
use crate::skin::Skin;
use crate::sprites::pl as p;
use crate::ui::{Control, PlayState, Ui, push_fmt, write_time};

pub fn draw(ui: &Ui, c: &mut Image, skin: &Skin, font: Option<&TtfFont>, r: Rect, now: Instant) {
    let (ox, oy, w, h) = (r.x, r.y, r.w, r.h);
    let bounds = c.bounds();
    let active = ui.active;
    let pressed = |ctl: Control| ui.pressed == Some(ctl);
    let sheet = skin.sheet(p::LEFT.sheet);
    // One scratch string reused for every piece of text drawn this frame.
    let mut text = String::with_capacity(96);

    // --- frame --------------------------------------------------------------------------
    // Top: corners, centred title piece, fill tiles between.
    let title_x = (w - 100) / 2;
    let fill = p::top_fill(active).rect();
    c.tile_h(sheet, fill, Rect::new(ox + 25, oy, title_x - 25, p::TOP_H));
    c.tile_h(
        sheet,
        fill,
        Rect::new(ox + title_x + 100, oy, w - 25 - (title_x + 100), p::TOP_H),
    );
    skin.blit(c, p::title(active), ox + title_x, oy);
    skin.blit(c, p::top_left(active), ox, oy);
    skin.blit(c, p::top_right(active), ox + w - 25, oy);
    if pressed(Control::PlClose) {
        skin.blit(
            c,
            p::CLOSE_PRESSED,
            ox + w - p::CLOSE_FROM_RIGHT,
            oy + p::TITLE_BTN_Y,
        );
    }
    if pressed(Control::PlShade) {
        skin.blit(
            c,
            p::SHADE_PRESSED,
            ox + w - p::SHADE_FROM_RIGHT,
            oy + p::TITLE_BTN_Y,
        );
    }

    // Sides.
    let side_h = h - p::TOP_H - p::BOTTOM_H;
    c.tile_v(
        sheet,
        p::LEFT.rect(),
        Rect::new(ox, oy + p::TOP_H, p::LEFT_W, side_h),
    );
    c.tile_v(
        sheet,
        p::RIGHT.rect(),
        Rect::new(ox + w - p::RIGHT_W, oy + p::TOP_H, p::RIGHT_W, side_h),
    );

    // Bottom.
    let by = oy + h - p::BOTTOM_H;
    c.tile_h(
        sheet,
        p::BOTTOM_FILL.rect(),
        Rect::new(ox + 125, by, w - 125 - 150, p::BOTTOM_H),
    );
    skin.blit(c, p::BOTTOM_LEFT, ox, by);
    skin.blit(c, p::BOTTOM_RIGHT, ox + w - 150, by);

    // --- track list ---------------------------------------------------------------------
    let content = ui.pl_content_rect(w, h).offset(ox, oy);
    let colors = &skin.pledit;
    c.fill_rect(content, colors.normal_bg);
    let list = ui.pl_list_rect(w, h).offset(ox, oy);
    let rows = usize::try_from((list.h + p::TRACK_H - 1) / p::TRACK_H).unwrap_or(0);
    let mut time = String::with_capacity(8);
    let current = ui.current_row();
    for (row, track) in ui.tracks.iter().enumerate().skip(ui.pl_scroll).take(rows) {
        let ry = list.y + i32::try_from(row - ui.pl_scroll).unwrap_or(0) * p::TRACK_H;
        if ui.selected.get(row).copied().unwrap_or(false) {
            c.fill_rect_clipped(
                Rect::new(list.x, ry, list.w, p::TRACK_H),
                colors.selected_bg,
                list,
            );
        }
        let color = if current == Some(row) {
            colors.current
        } else {
            colors.normal
        };
        text.clear();
        push_fmt(&mut text, format_args!("{}. {}", row + 1, track.title));
        // Unknown lengths stay blank until the file has been read.
        time.clear();
        if let Some(secs) = track.seconds {
            write_time(&mut time, secs);
        }
        match font {
            Some(f) => {
                let tw = f.width(&time);
                f.draw(c, &time, list.right() - 3 - tw, ry + 1, color, list);
                let label_clip =
                    Rect::new(list.x, ry, list.w - tw - 6, p::TRACK_H).intersect(&list);
                f.draw(c, &text, list.x, ry + 1, color, label_clip);
            }
            None => {
                let tw = bitmap_text_width(&time);
                draw_bitmap_text(c, skin, &time, list.right() - 3 - tw, ry + 3, list);
                let label_clip =
                    Rect::new(list.x, ry, list.w - tw - 6, p::TRACK_H).intersect(&list);
                draw_bitmap_text(c, skin, &text, list.x, ry + 3, label_clip);
            }
        }
    }

    // Scrollbar thumb.
    let max_scroll = ui.max_scroll();
    let f = if max_scroll == 0 {
        0.0
    } else {
        ui.pl_scroll as f32 / max_scroll as f32
    };
    let ty = content.y + (f * (content.h - p::SCROLL_THUMB_H) as f32).round() as i32;
    let thumb = if pressed(Control::PlScroll) {
        p::SCROLL_THUMB_PRESSED
    } else {
        p::SCROLL_THUMB
    };
    skin.blit(c, thumb, ox + w - p::SCROLL_FROM_RIGHT, ty);

    // --- bottom-right readouts ----------------------------------------------------------
    let br = (ox + w - 150, by);
    let selected_secs: u32 = ui
        .tracks
        .iter()
        .zip(&ui.selected)
        .filter(|(_, s)| **s)
        .filter_map(|(t, _)| t.seconds)
        .sum();
    let total_secs: u32 = ui.tracks.iter().filter_map(|t| t.seconds).sum();
    text.clear();
    write_time(&mut text, selected_secs);
    text.push('/');
    write_time(&mut text, total_secs);
    draw_bitmap_text(
        c,
        skin,
        &text,
        br.0 + p::RUNNING_TIME_POS.0,
        br.1 + p::RUNNING_TIME_POS.1,
        bounds,
    );

    if ui.state != PlayState::Stopped && !ui.blink_off(now) {
        let (mins, secs, neg) = ui.display_time();
        text.clear();
        push_fmt(
            &mut text,
            format_args!("{}{mins:02}", if neg { "-" } else { "" }),
        );
        let mw = bitmap_text_width(&text);
        let y = br.1 + p::MINI_TIME_Y;
        draw_bitmap_text(c, skin, &text, br.0 + p::MINI_TIME_MIN_END - mw, y, bounds);
        text.clear();
        push_fmt(&mut text, format_args!("{secs:02}"));
        draw_bitmap_text(c, skin, &text, br.0 + p::MINI_TIME_SEC_X, y, bounds);
    }

    // --- the menu of the bottom button held ---------------------------------------------
    if let Some(menu) = ui.pl_menu() {
        let (button, items) = menu.rects(w, h);
        let (items_x, bar_x) = menu.sprite_columns();
        let n = i32::try_from(menu.items().len()).unwrap_or(0);
        skin.blit(c, p::menu_bar(bar_x, n), ox + button.x, oy + items.y);
        for (i, it) in (0..).zip(menu.items()) {
            let item = Rect::new(
                items.x,
                items.y + i * p::MENU_ITEM_H,
                p::MENU_ITEM_W,
                p::MENU_ITEM_H,
            );
            let hot = item.contains(ui.cursor.0, ui.cursor.1);
            let sprite = p::menu_item(items_x, it.sprite_row(), hot);
            skin.blit(c, sprite, ox + item.x, oy + item.y);
        }
    }
}
