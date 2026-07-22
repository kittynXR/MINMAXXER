fn main() {
    ensure_windows_icon();
    tauri_build::build()
}

fn ensure_windows_icon() {
    let path = std::path::Path::new("icons/icon.ico");
    if path.exists() {
        return;
    }
    let _ = std::fs::create_dir_all("icons");
    let width = 32_u32;
    let height = 32_u32;
    let pixel_bytes = (width * height * 4) as usize;
    let mask_stride = width.div_ceil(32) * 4;
    let mask_bytes = (mask_stride * height) as usize;
    let image_bytes = 40 + pixel_bytes + mask_bytes;
    let mut icon = Vec::with_capacity(22 + image_bytes);
    icon.extend_from_slice(&0_u16.to_le_bytes());
    icon.extend_from_slice(&1_u16.to_le_bytes());
    icon.extend_from_slice(&1_u16.to_le_bytes());
    icon.extend_from_slice(&[width as u8, height as u8, 0, 0]);
    icon.extend_from_slice(&1_u16.to_le_bytes());
    icon.extend_from_slice(&32_u16.to_le_bytes());
    icon.extend_from_slice(&(image_bytes as u32).to_le_bytes());
    icon.extend_from_slice(&22_u32.to_le_bytes());
    icon.extend_from_slice(&40_u32.to_le_bytes());
    icon.extend_from_slice(&(width as i32).to_le_bytes());
    icon.extend_from_slice(&((height * 2) as i32).to_le_bytes());
    icon.extend_from_slice(&1_u16.to_le_bytes());
    icon.extend_from_slice(&32_u16.to_le_bytes());
    icon.extend_from_slice(&0_u32.to_le_bytes());
    icon.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    icon.extend_from_slice(&0_i32.to_le_bytes());
    icon.extend_from_slice(&0_i32.to_le_bytes());
    icon.extend_from_slice(&0_u32.to_le_bytes());
    icon.extend_from_slice(&0_u32.to_le_bytes());
    for source_y in 0..height {
        let y = height - 1 - source_y;
        for x in 0..width {
            let dx = x as i32 - 16;
            let dy = y as i32 - 16;
            let inside = dx * dx + dy * dy <= 15 * 15;
            let mark = ((7..=11).contains(&x) || (21..=25).contains(&x)) && (8..=24).contains(&y)
                || (x as i32 - 16).abs() <= 2 && (10..=20).contains(&y);
            let (red, green, blue, alpha) = if inside && mark {
                (114, 239, 207, 255)
            } else if inside {
                (11, 18, 34, 255)
            } else {
                (0, 0, 0, 0)
            };
            icon.extend_from_slice(&[blue, green, red, alpha]);
        }
    }
    for source_y in 0..height {
        let y = height - 1 - source_y;
        let mut row = vec![0_u8; mask_stride as usize];
        for x in 0..width {
            let dx = x as i32 - 16;
            let dy = y as i32 - 16;
            if dx * dx + dy * dy > 15 * 15 {
                row[(x / 8) as usize] |= 1 << (7 - (x % 8));
            }
        }
        icon.extend_from_slice(&row);
    }
    std::fs::write(path, icon).expect("failed generating Windows application icon");
}
