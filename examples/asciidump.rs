//! Dev tool: ASCII color-map dump of `preview.png` for headless visual QA.
//! Run: `nix-shell shell.nix --run 'cargo run --example asciidump'`

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "preview.png".into());
    let img = std::fs::File::open(&path).unwrap();
    let decoder = png::Decoder::new(img);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    let (w, h) = (info.width as usize, info.height as usize);
    let bytes = &buf[..info.buffer_size()];
    let sb = 12usize;

    let cls = |p: &[u8]| -> char {
        let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
        if r < 10 && g < 10 && b < 10 {
            '.'
        } else if (r - 28).abs() < 9 && (g - 28).abs() < 9 && (b - 28).abs() < 9 {
            'K'
        } else if (r - 13).abs() < 7 && (g - 13).abs() < 7 && (b - 13).abs() < 7 {
            'S'
        } else if r > 200 && g > 200 && b > 200 {
            '#'
        } else if (r - 140).abs() < 30 && (g - 140).abs() < 30 && (b - 140).abs() < 30 {
            's'
        } else {
            '?'
        }
    };

    let mut prev = String::new();
    let mut y = 0usize;
    while y < h {
        let mut row = String::new();
        let mut x = 0usize;
        while x < w {
            let i = (y * w + x) * 4;
            if i + 4 <= bytes.len() {
                row.push(cls(&bytes[i..i + 4]));
            }
            x += sb;
        }
        if row != prev || row.contains('?') {
            println!("y={y:4} {row}");
            prev = row;
        }
        y += sb;
    }
}