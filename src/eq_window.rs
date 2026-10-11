//! Drawing of the 275x116 equalizer window.

use std::time::Instant;

use crate::canvas::{Image, Rect};
use crate::skin::Skin;
use crate::sprites::eq as e;
use crate::ui::{Control, Ui};

pub fn draw(ui: &Ui, c: &mut Image, skin: &Skin, r: Rect, _now: Instant) {
    let (ox, oy) = (r.x, r.y);
    let pressed = |ctl: Control| ui.pressed == Some(ctl);

    skin.blit(c, e::BACKGROUND, ox, oy);
    skin.blit(
        c,
        if ui.active {
            e::TITLEBAR
        } else {
            e::TITLEBAR_INACTIVE
        },
        ox,
        oy,
    );
    if pressed(Control::EqClose) {
        skin.blit(c, e::CLOSE_PRESSED, ox + e::CLOSE.x, oy + e::CLOSE.y);
    }
    if pressed(Control::EqShade) {
        skin.blit(c, e::SHADE_PRESSED, ox + e::SHADE.x, oy + e::SHADE.y);
    }

    skin.blit(
        c,
        e::on_sprite(ui.eq_on, pressed(Control::EqOn)),
        ox + e::ON.x,
        oy + e::ON.y,
    );
    skin.blit(
        c,
        e::auto_sprite(ui.eq_auto, pressed(Control::EqAuto)),
        ox + e::AUTO.x,
        oy + e::AUTO.y,
    );
    let presets = if pressed(Control::EqPresets) {
        e::PRESETS_DOWN
    } else {
        e::PRESETS_UP
    };
    skin.blit(c, presets, ox + e::PRESETS.x, oy + e::PRESETS.y);

    draw_graph(ui, c, skin, e::GRAPH.offset(ox, oy));

    draw_slider(
        c,
        skin,
        e::PREAMP.offset(ox, oy),
        ui.preamp,
        pressed(Control::EqPreamp),
    );
    for (i, v) in ui.bands.iter().enumerate() {
        let band = i32::try_from(i).unwrap_or(0);
        draw_slider(
            c,
            skin,
            e::band(band).offset(ox, oy),
            *v,
            pressed(Control::EqBand(i)),
        );
    }
}

fn draw_slider(c: &mut Image, skin: &Skin, r: Rect, value: f32, pressed: bool) {
    // `value` is clamped to 0..=1, so these casts stay in range.
    let frame = (value * (e::SLIDER_FRAMES - 1) as f32).round() as i32;
    skin.blit(c, e::slider_frame(frame), r.x, r.y);
    let ty = ((1.0 - value) * e::SLIDER_TRAVEL as f32) as i32;
    let thumb = if pressed {
        e::SLIDER_THUMB_PRESSED
    } else {
        e::SLIDER_THUMB
    };
    skin.blit(c, thumb, r.x + 1, r.y + ty);
}

/// The 113x19 response curve: a spline through the ten band values, coloured by height.
fn draw_graph(ui: &Ui, c: &mut Image, skin: &Skin, r: Rect) {
    const ROWS: usize = 19;
    skin.blit(c, e::GRAPH_BG, r.x, r.y);
    let gradient = skin.sheet(e::GRAPH_GRADIENT.sheet);
    let colors: [[u8; 4]; ROWS] =
        std::array::from_fn(|i| gradient.get(e::GRAPH_GRADIENT.x, e::GRAPH_GRADIENT.y + i as i32));

    let py = ((1.0 - ui.preamp) * (ROWS - 1) as f32).round() as i32;
    skin.blit(c, e::PREAMP_LINE, r.x, r.y + py);

    let ys: [f32; 10] = ui.bands.map(|v| (1.0 - v) * (ROWS - 1) as f32);
    let n = ys.len();
    let step = (r.w - 1) as f32 / (n - 1) as f32;
    for x in 0..r.w {
        let fx = x as f32 / step;
        let i = (fx.floor() as usize).min(n - 2);
        let t = fx - i as f32;
        let p0 = ys[i.saturating_sub(1)];
        let p1 = ys[i];
        let p2 = ys[i + 1];
        let p3 = ys[(i + 2).min(n - 1)];
        // Catmull-Rom spline.
        let y = 0.5
            * ((2.0 * p1)
                + (-p0 + p2) * t
                + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
                + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t);
        let yi = y.round().clamp(0.0, (ROWS - 1) as f32) as usize;
        c.put(r.x + x, r.y + yi as i32, colors[yi]);
    }
}
