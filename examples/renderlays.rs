//! Dev tool: renders every keyboard layer headlessly to PNGs for visual QA.
//! Run: `nix-shell shell.nix --run 'cargo run --example renderlays'`

use hyprosk::layout::{KeyboardLayout, LayerId};
use hyprosk::render::slint::SlintScene;
use hyprosk::render::theme::Theme;

const W: u32 = 1440;
const H: u32 = 414;

fn render(scene: &mut SlintScene, layout: &KeyboardLayout, name: &str) {
    let theme = Theme::catppuccin_mocha();
    let mut argb = vec![0u8; (W * H * 4) as usize];
    let ok = scene.render(layout, &theme, W, H, None, None, argb.as_mut());
    assert!(ok, "render {name} failed");

    let mut rgba = Vec::with_capacity((W * H * 4) as usize);
    for px in argb.chunks_exact(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    let path = format!("lay-{name}.png");
    let mut file = std::fs::File::create(&path).unwrap();
    let mut enc = png::Encoder::new(&mut file, W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgba).unwrap();
    println!("wrote {path}");
}

fn main() {
    let mut scene = SlintScene::new(W, H).expect("SlintScene::new");
    let sugg: Vec<String> = vec!["thanks".into(), "thank you".into(), "thanks bro".into()];
    render(&mut scene, &KeyboardLayout::get_layout(LayerId::Lower, &sugg), "letters");
    render(&mut scene, &KeyboardLayout::get_layout(LayerId::Lower, &[]), "letters-idle");
    render(&mut scene, &KeyboardLayout::get_layout(LayerId::Upper, &[]), "upper");
    render(&mut scene, &KeyboardLayout::get_layout(LayerId::Symbols, &[]), "symbols1");
    render(&mut scene, &KeyboardLayout::get_layout(LayerId::Symbols2, &[]), "symbols2");

    let history: Vec<String> = vec![
        "Let's deploy the project update tomorrow at 10:00 AM!".into(),
        "git clone https://github.com/microsoft/fluentui.git".into(),
        "https://github.com/anomalyco/opencode".into(),
        "contact.support@windows11.design".into(),
        "Thanks bro! Everything is working smoothly now.".into(),
        "#0078D4".into(),
        "cd ~/repo/HyprOsk && cargo run".into(),
        "hello world".into(),
    ];
    render(&mut scene, &KeyboardLayout::clipboard(&history, &[]), "clipboard");

    let hist1: Vec<String> = vec!["only item".into()];
    render(&mut scene, &KeyboardLayout::clipboard(&hist1, &[]), "clipboard-one");
}