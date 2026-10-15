//! Drawing of the 275x116 main window.

use std::time::Instant;

use crate::canvas::{Image, Rect};
use crate::font::{draw_bitmap_chars, draw_bitmap_text};
use crate::skin::Skin;
use crate::sprites::{Sheet, digits, main as m};
use crate::ui::{Control, PlayState, Ui, push_fmt};

pub fn draw(ui: &Ui, c: &mut Image, skin: &Skin, r: Rect, now: Instant) {
    let (ox, oy) = (r.x, r.y);
    let bounds = c.bounds();
    let pressed = |ctl: Control| ui.pressed == Some(ctl);

    skin.blit(c, m::BACKGROUND, ox, oy);
    skin.blit(
        c,
        if ui.active {
            m::TITLEBAR
        } else {
            m::TITLEBAR_INACTIVE
        },
        ox,
        oy,
    );

    // Title bar buttons only have "pressed" sprites; the rest is baked into the bar.
    for (ctl, rect, sprite) in [
        (Control::MainOptions, m::OPTIONS, m::OPTIONS_PRESSED),
        (Control::MainMinimize, m::MINIMIZE, m::MINIMIZE_PRESSED),
        (Control::MainShade, m::SHADE, m::SHADE_PRESSED),
        (Control::MainClose, m::CLOSE, m::CLOSE_PRESSED),
    ] {
        if pressed(ctl) {
            skin.blit(c, sprite, ox + rect.x, oy + rect.y);
        }
    }

    // Clutter bar (O A I D V). A and D stay down while always on top and double size are on.
    skin.blit(c, m::CLUTTER_BG, ox + m::CLUTTER.x, oy + m::CLUTTER.y);
    for (ctl, rect, sprite, on) in [
        (Control::ClutterO, m::CLUTTER_O, m::CLUTTER_O_PRESSED, false),
        (
            Control::ClutterA,
            m::CLUTTER_A,
            m::CLUTTER_A_PRESSED,
            ui.always_on_top,
        ),
        (Control::ClutterI, m::CLUTTER_I, m::CLUTTER_I_PRESSED, false),
        (
            Control::ClutterD,
            m::CLUTTER_D,
            m::CLUTTER_D_PRESSED,
            ui.double_size,
        ),
        (Control::ClutterV, m::CLUTTER_V, m::CLUTTER_V_PRESSED, false),
    ] {
        if pressed(ctl) || on {
            skin.blit(c, sprite, ox + rect.x, oy + rect.y);
        }
    }

    // Play / pause / stop status and the "working" indicator.
    let status = match ui.state {
        PlayState::Playing => m::STATUS_PLAY,
        PlayState::Paused => m::STATUS_PAUSE,
        PlayState::Stopped => m::STATUS_STOP,
    };
    skin.blit(c, status, ox + m::STATUS_POS.0, oy + m::STATUS_POS.1);
    if ui.state == PlayState::Playing {
        skin.blit(
            c,
            m::NOT_WORKING,
            ox + m::WORK_INDICATOR_POS.0,
            oy + m::WORK_INDICATOR_POS.1,
        );
    }

    // LED time display. Blinks while paused, blank while stopped.
    if ui.state != PlayState::Stopped && !ui.blink_off(now) {
        let (mins, secs, neg) = ui.display_time();
        let ex = skin.has_sheet(Sheet::NumsEx);
        // Each value is a single decimal digit, so the narrowing is lossless.
        let digit = |d: u32| {
            let d = u8::try_from(d % 10).unwrap_or(0);
            if ex {
                digits::digit_ex(d)
            } else {
                digits::digit(d)
            }
        };
        if neg {
            if ex {
                skin.blit(
                    c,
                    digits::MINUS_EX,
                    ox + m::MINUS_POS.0,
                    oy + m::MINUS_POS.1,
                );
            } else {
                draw_minus(c, skin, ox + m::MINUS_POS.0, oy + m::MINUS_POS.1);
            }
        }
        let ds = [(mins / 10) % 10, mins % 10, secs / 10, secs % 10];
        for (i, d) in ds.iter().enumerate() {
            skin.blit(c, digit(*d), ox + m::DIGIT_X[i], oy + m::DIGIT_Y);
        }
    }

    // Song title marquee (no per-frame allocation: the title is cached as chars).
    let marquee = m::MARQUEE.offset(ox, oy);
    match &ui.marquee_override {
        Some(s) => draw_bitmap_chars(c, skin, s.chars(), marquee.x, marquee.y, marquee),
        None => draw_bitmap_chars(
            c,
            skin,
            ui.title_scroll_chars(now),
            marquee.x,
            marquee.y,
            marquee,
        ),
    }

    // Bitrate / sample rate / channels.
    if ui.state != PlayState::Stopped {
        let mut text = String::with_capacity(4);
        push_fmt(&mut text, format_args!("{}", ui.kbps.min(999)));
        draw_bitmap_text(
            c,
            skin,
            &text,
            ox + m::KBPS_POS.0,
            oy + m::KBPS_POS.1,
            bounds,
        );
        text.clear();
        push_fmt(&mut text, format_args!("{}", ui.khz.min(99)));
        draw_bitmap_text(c, skin, &text, ox + m::KHZ_POS.0, oy + m::KHZ_POS.1, bounds);
    }
    let playing = ui.state != PlayState::Stopped;
    skin.blit(
        c,
        if playing && !ui.stereo {
            m::MONO_ON
        } else {
            m::MONO_OFF
        },
        ox + m::MONO.x,
        oy + m::MONO.y,
    );
    skin.blit(
        c,
        if playing && ui.stereo {
            m::STEREO_ON
        } else {
            m::STEREO_OFF
        },
        ox + m::STEREO.x,
        oy + m::STEREO.y,
    );

    // Visualiser.
    if ui.state != PlayState::Stopped {
        ui.vis.draw(c, skin, m::VIS.offset(ox, oy));
    }

    // Volume and balance sliders: 28 background frames plus a thumb.
    // Slider values are clamped to 0..=1 (balance to -1..=1), so these casts stay in range.
    let vframe = (ui.volume * (m::SLIDER_FRAMES - 1) as f32).round() as i32;
    skin.blit(
        c,
        m::volume_frame(vframe),
        ox + m::VOLUME.x,
        oy + m::VOLUME.y,
    );
    let vx = (ui.volume * (m::VOLUME.w - m::SLIDER_THUMB_W) as f32).round() as i32;
    let vthumb = if pressed(Control::Volume) {
        m::VOLUME_THUMB_PRESSED
    } else {
        m::VOLUME_THUMB
    };
    skin.blit(c, vthumb, ox + m::VOLUME.x + vx, oy + m::VOLUME.y + 1);

    let bframe = (ui.balance.abs() * (m::SLIDER_FRAMES - 1) as f32).round() as i32;
    skin.blit(
        c,
        m::balance_frame(bframe),
        ox + m::BALANCE.x,
        oy + m::BALANCE.y,
    );
    let bx = ((ui.balance + 1.0) / 2.0 * (m::BALANCE.w - m::SLIDER_THUMB_W) as f32).round() as i32;
    let bthumb = if pressed(Control::Balance) {
        m::BALANCE_THUMB_PRESSED
    } else {
        m::BALANCE_THUMB
    };
    skin.blit(c, bthumb, ox + m::BALANCE.x + bx, oy + m::BALANCE.y + 1);

    // Seek bar; the thumb is hidden while stopped.
    skin.blit(c, m::POSBAR_BG, ox + m::POSBAR.x, oy + m::POSBAR.y);
    if ui.state != PlayState::Stopped {
        let px = (ui.position() * (m::POSBAR.w - m::POSBAR_THUMB_W) as f32).round() as i32;
        let thumb = if pressed(Control::PosBar) {
            m::POSBAR_THUMB_PRESSED
        } else {
            m::POSBAR_THUMB
        };
        skin.blit(c, thumb, ox + m::POSBAR.x + px, oy + m::POSBAR.y);
    }

    // Transport buttons.
    skin.blit(
        c,
        m::prev_sprite(pressed(Control::Prev)),
        ox + m::PREV.x,
        oy + m::PREV.y,
    );
    skin.blit(
        c,
        m::play_sprite(pressed(Control::Play)),
        ox + m::PLAY.x,
        oy + m::PLAY.y,
    );
    skin.blit(
        c,
        m::pause_sprite(pressed(Control::Pause)),
        ox + m::PAUSE.x,
        oy + m::PAUSE.y,
    );
    skin.blit(
        c,
        m::stop_sprite(pressed(Control::Stop)),
        ox + m::STOP.x,
        oy + m::STOP.y,
    );
    skin.blit(
        c,
        m::next_sprite(pressed(Control::Next)),
        ox + m::NEXT.x,
        oy + m::NEXT.y,
    );
    skin.blit(
        c,
        m::eject_sprite(pressed(Control::Eject)),
        ox + m::EJECT.x,
        oy + m::EJECT.y,
    );

    // Toggles.
    skin.blit(
        c,
        m::shuffle_sprite(ui.shuffle, pressed(Control::Shuffle)),
        ox + m::SHUFFLE.x,
        oy + m::SHUFFLE.y,
    );
    skin.blit(
        c,
        m::repeat_sprite(ui.repeat, pressed(Control::Repeat)),
        ox + m::REPEAT.x,
        oy + m::REPEAT.y,
    );
    skin.blit(
        c,
        m::eq_btn_sprite(ui.eq_open, pressed(Control::EqToggle)),
        ox + m::EQ_BTN.x,
        oy + m::EQ_BTN.y,
    );
    skin.blit(
        c,
        m::pl_btn_sprite(ui.pl_open, pressed(Control::PlToggle)),
        ox + m::PL_BTN.x,
        oy + m::PL_BTN.y,
    );
}

/// Skins without nums_ex.bmp have no minus glyph: draw a bar in the digit colour.
fn draw_minus(c: &mut Image, skin: &Skin, x: i32, y: i32) {
    skin.blit(c, digits::BLANK, x, y);
    let sheet = skin.sheet(Sheet::Numbers);
    // Borrow the colour of the horizontal segment of "0" (top row of the glyph).
    let mut color = [0, 255, 0, 255];
    for sx in 0..9 {
        let px = sheet.get(sx, 0);
        if px[0] as u32 + px[1] as u32 + px[2] as u32 > 160 {
            color = px;
            break;
        }
    }
    c.fill_rect(Rect::new(x + 2, y + 6, 5, 1), color);
}
