//! Render exact small-screen smartphone preview (390x280)
//! (e.g. iPhone / Pixel standard portrait viewport).

use hyprosk::layout::{KeyboardLayout, LayerId};
use hyprosk::render::slint::SlintScene;
use hyprosk::render::theme::Theme;

const W: u32 = 390;
const H: u32 = 280;

fn save_png(filename: &str, argb: &[u8], width: u32, height: u32) {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for px in argb.chunks_exact(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }

    let mut file = std::fs::File::create(filename).unwrap();
    let mut enc = png::Encoder::new(&mut file, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgba).unwrap();
    println!("Saved phone preview: {filename} ({width}x{height})");
}

fn main() {
    let suggestions: Vec<String> = vec!["the".into(), "hello".into(), "world".into()];
    let theme = Theme::catppuccin_mocha();

    let layers = [
        (LayerId::Lower, false, "preview_phone_lower.png", "Phone Letters Lower (QWERTY)"),
        (LayerId::Upper, false, "preview_phone_upper.png", "Phone Letters Upper (Shift)"),
        (LayerId::Symbols, false, "preview_phone_symbols.png", "Phone Symbols Page 1"),
        (LayerId::Symbols2, false, "preview_phone_symbols2.png", "Phone Symbols Page 2"),
    ];

    let mut scene = SlintScene::new(W, H).expect("SlintScene::new");

    for (layer_id, caps, filename, title) in layers {
        let layout = KeyboardLayout::get_portrait_layout_with_caps(layer_id, &suggestions, caps);
        let mut argb = vec![0u8; (W * H * 4) as usize];
        let damage = scene.render(
            &layout,
            &theme,
            W,
            H,
            &[],
            &[],
            None,
            None,
            argb.as_mut(),
        );
        assert!(damage.is_some(), "Rendering failed for layer {title}");
        save_png(filename, &argb, W, H);
    }

    println!("All small-screen phone previews generated successfully!");
}
