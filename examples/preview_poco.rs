//! Render exact Poco X3 Pro smartphone viewport (393x315 logical viewport, 20:9 aspect).

use hyprosk::layout::{KeyboardLayout, LayerId};
use hyprosk::render::slint::SlintScene;
use hyprosk::render::theme::Theme;

// Poco X3 Pro: 1080x2400 physical display, 2.75x display scaling factor
// Logical portrait width = 393px, typical Gboard keyboard height = 315px
const W: u32 = 393;
const H: u32 = 315;

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
    println!("Saved Poco X3 Pro preview: {filename} ({width}x{height})");
}

fn main() {
    let suggestions: Vec<String> = vec!["the".into(), "hello".into(), "world".into()];
    let theme = Theme::catppuccin_mocha();

    let layers = [
        (LayerId::Lower, false, "preview_poco_lower.png", "Poco X3 Pro Lower (QWERTY)"),
        (LayerId::Upper, false, "preview_poco_upper.png", "Poco X3 Pro Upper (Shift)"),
        (LayerId::Symbols, false, "preview_poco_symbols.png", "Poco X3 Pro Symbols Page 1"),
        (LayerId::Symbols2, false, "preview_poco_symbols2.png", "Poco X3 Pro Symbols Page 2"),
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

    println!("All Poco X3 Pro previews generated successfully!");
}
