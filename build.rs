fn main() {
    ensure_icon();
    tauri_build::build()
}

fn ensure_icon() {
    use std::{fs, fs::File, io::BufWriter, path::Path};

    let path = Path::new("icons/icon.png");
    if path.exists() {
        return;
    }
    fs::create_dir_all("icons").expect("create icon directory");
    let file = File::create(path).expect("create EmuMi icon");
    let mut encoder = png::Encoder::new(BufWriter::new(file), 64, 64);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write icon header");

    let mut pixels = vec![0_u8; 64 * 64 * 4];
    for y in 0_usize..64 {
        for x in 0_usize..64 {
            let i = (y * 64 + x) * 4;
            let rounded = (x < 8 && y < 8 && (8 - x) * (8 - x) + (8 - y) * (8 - y) > 64)
                || (x > 55 && y < 8 && (x - 55) * (x - 55) + (8 - y) * (8 - y) > 64)
                || (x < 8 && y > 55 && (8 - x) * (8 - x) + (y - 55) * (y - 55) > 64)
                || (x > 55 && y > 55 && (x - 55) * (x - 55) + (y - 55) * (y - 55) > 64);
            if !rounded {
                pixels[i..i + 4].copy_from_slice(&[25, 132, 78, 255]);
            }
        }
    }

    let white = [255, 255, 255, 255];
    let paint = |pixels: &mut [u8], x: usize, y: usize| {
        let i = (y * 64 + x) * 4;
        pixels[i..i + 4].copy_from_slice(&white);
    };
    for y in 25..47 {
        for x in 16..48 {
            if (x >= 18 && x <= 45) || (y >= 28 && y <= 43) {
                paint(&mut pixels, x, y);
            }
        }
    }
    for y in 18..30 {
        for x in 18..46 {
            let dx = x as isize - 32;
            let dy = y as isize - 30;
            if dx * dx + dy * dy <= 16 * 16 {
                paint(&mut pixels, x, y);
            }
        }
    }
    for (x, y) in [(25, 24), (39, 24)] {
        for yy in y..y + 3 {
            for xx in x..x + 3 {
                let i = (yy * 64 + xx) * 4;
                pixels[i..i + 4].copy_from_slice(&[25, 132, 78, 255]);
            }
        }
    }
    for step in 0..11 {
        paint(&mut pixels, 21 - step / 2, 18 - step);
        paint(&mut pixels, 43 + step / 2, 18 - step);
    }

    writer.write_image_data(&pixels).expect("write EmuMi icon");
}
