//! Visual QA preview: renders the Wireframe scene headlessly to `preview.png`
//! (lowercase QWERTY with suggestions) without needing a Wayland compositor.
//! Also probes pixels (via the shared `RenderEngine` geometry) and exits
//! non-zero if the scene doesn't match expected colors, mirroring the spike.
//!
//! Run: `nix-shell shell.nix --run 'cargo run --example preview'`

use hyprosk::layout::{KeyboardLayout, LayerId};
use hyprosk::render::engine::RenderEngine;
use hyprosk::render::slint::SlintScene;
use hyprosk::render::theme::Theme;

const W: u32 = 1200;
const H: u32 = 420;

fn px(argb: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = (y * W + x) as usize * 4;
    [argb[i + 2], argb[i + 1], argb[i], argb[i + 3]] // r,g,b,a
}

fn close(a: [u8; 4], e: [u8; 4], tol: i32) -> bool {
    a.iter()
        .zip(e.iter())
        .all(|(a, e)| (*a as i32 - *e as i32).abs() <= tol)
}

fn main() {
    let suggestions: Vec<String> = vec!["thanks".into(), "bro".into(), "beech".into()];
    let layout = KeyboardLayout::get_layout(LayerId::Lower, &suggestions);
    let theme = Theme::catppuccin_mocha();
    let rects = RenderEngine::calculate_key_rects(&layout, W, H, &theme);

    let mut scene = SlintScene::new(W, H).expect("SlintScene::new");
    let mut argb = vec![0u8; (W * H * 4) as usize];
    let ok = scene.render(&layout, &theme, W, H, Some((2, 0)), None, argb.as_mut());
    println!("render ok: {ok}");
    assert!(ok);

    let mut fails = 0usize;
    // 1. Rounded top-left corner is fully transparent (alpha 0).
    let corner = px(&argb, 2, 2);
    if close(corner, [0, 0, 0, 0], 8) {
        println!("corner transparent -> PASS");
    } else {
        println!("corner alpha FAIL: {corner:?}");
        fails += 1;
    }

    // 2. Dock background (#1a1a1a) visible in the gap between key rows.
    let gap_y = (rects[1][0].0.y + rects[1][0].0.h + 3.0) as u32;
    let dock = px(&argb, rects[1][0].0.x as u32 + 2, gap_y);
    if close(dock, [26, 26, 26, 255], 8) {
        println!("dock background -> PASS");
    } else {
        println!("dock bg FAIL: got {dock:?}");
        fails += 1;
    }

    // 3. A regular letter key center matches wireframe key_background #2d2d2d.
    let key_mid = &rects[1][2].0; // 'w' key
    let mid = px(&argb, (key_mid.x + key_mid.w / 2.0) as u32, (key_mid.y + key_mid.h / 4.0) as u32);
    if close(mid, [45, 45, 45, 255], 10) {
        println!("letter key background -> PASS");
    } else {
        println!("letter key bg FAIL: {mid:?}");
        fails += 1;
    }

    // 4. Pressed key (Tab, row 2 col 0) shows the pressed tint (#1e1e1e).
    let tab = &rects[2][0].0;
    let pressed = px(&argb, (tab.x + tab.w / 2.0) as u32, (tab.y + tab.h / 3.0) as u32);
    if close(pressed, [30, 30, 30, 255], 12) {
        println!("pressed key tint -> PASS");
    } else {
        println!("pressed-key tint FAIL: {pressed:?}");
        fails += 1;
    }

    // Convert ARGB-over-memory (B,G,R,A bytes) to RGBA for PNG.
    let mut rgba = Vec::with_capacity((W * H * 4) as usize);
    for px in argb.chunks_exact(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }

    let mut file = std::fs::File::create("preview.png").unwrap();
    let mut enc = png::Encoder::new(&mut file, W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgba).unwrap();
    println!("wrote preview.png {W}x{H}");

    println!("PREVIEW RESULT: {fails} failure(s)");
    assert_eq!(fails, 0);
}