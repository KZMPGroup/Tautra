const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;
const MAX_PIXELS: u64 = 128 * 1024 * 1024;
const DECODE_LIMIT: usize = 1024 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("format bitmap tidak didukung: {0}")]
    Unsupported(String),
    #[error("data bitmap rusak: {0}")]
    Corrupt(&'static str),
    #[error("gagal encode PNG: {0}")]
    Encode(#[from] png::EncodingError),
    #[error("gagal decode PNG: {0}")]
    Decode(#[from] png::DecodingError),
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(offset..offset + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(offset..offset + 4)?.try_into().ok()?))
}

fn channel(px: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones();
    let value = (px & mask) >> shift;
    if bits >= 8 {
        (value >> (bits - 8)) as u8
    } else {
        (value * 255 / ((1u32 << bits) - 1)) as u8
    }
}

pub fn dib_to_png(dib: &[u8]) -> Result<Vec<u8>, ImageError> {
    let corrupt = ImageError::Corrupt;
    let header = u32_at(dib, 0).ok_or(corrupt("header terlalu pendek"))? as usize;
    if header < 40 {
        return Err(ImageError::Unsupported(format!("header {header} byte")));
    }
    let width = u32_at(dib, 4).ok_or(corrupt("lebar"))? as i32;
    let height = u32_at(dib, 8).ok_or(corrupt("tinggi"))? as i32;
    let bpp = u16_at(dib, 14).ok_or(corrupt("bit per piksel"))?;
    let compression = u32_at(dib, 16).ok_or(corrupt("kompresi"))?;
    let clr_used = u32_at(dib, 32).ok_or(corrupt("palet"))? as usize;
    if width <= 0 || height == 0 {
        return Err(corrupt("ukuran gambar nol"));
    }
    let top_down = height < 0;
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    if (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(ImageError::Unsupported(format!("{w}×{h} piksel terlalu besar")));
    }
    if bpp != 24 && bpp != 32 {
        return Err(ImageError::Unsupported(format!("{bpp} bit per piksel")));
    }
    let (masks, extra) = match compression {
        BI_RGB => ((0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000), 0),
        BI_BITFIELDS | BI_ALPHABITFIELDS => {
            let inline = header >= 52;
            let base = 40;
            let r = u32_at(dib, base).ok_or(corrupt("mask merah"))?;
            let g = u32_at(dib, base + 4).ok_or(corrupt("mask hijau"))?;
            let b = u32_at(dib, base + 8).ok_or(corrupt("mask biru"))?;
            let has_alpha = compression == BI_ALPHABITFIELDS || header >= 56;
            let a = if has_alpha {
                u32_at(dib, base + 12).unwrap_or(0)
            } else {
                0
            };
            let extra = if inline {
                0
            } else if compression == BI_ALPHABITFIELDS {
                16
            } else {
                12
            };
            ((r, g, b, a), extra)
        }
        other => return Err(ImageError::Unsupported(format!("kompresi {other}"))),
    };
    let offset = header + extra + clr_used * 4;
    let stride = (w * bpp as usize).div_ceil(32) * 4;
    if dib.len() < offset + stride * h {
        return Err(corrupt("data piksel terpotong"));
    }
    let pixels = &dib[offset..];
    let mut rgba = vec![0u8; w * h * 4];
    let mut any_alpha = false;
    for row in 0..h {
        let src_row = if top_down { row } else { h - 1 - row };
        let src = &pixels[src_row * stride..src_row * stride + stride];
        let dst = &mut rgba[row * w * 4..(row + 1) * w * 4];
        for x in 0..w {
            let out = &mut dst[x * 4..x * 4 + 4];
            if bpp == 24 {
                let p = &src[x * 3..x * 3 + 3];
                out.copy_from_slice(&[p[2], p[1], p[0], 255]);
            } else {
                let px = u32::from_le_bytes(src[x * 4..x * 4 + 4].try_into().unwrap_or_default());
                let a = channel(px, masks.3);
                any_alpha |= a != 0;
                out.copy_from_slice(&[
                    channel(px, masks.0),
                    channel(px, masks.1),
                    channel(px, masks.2),
                    a,
                ]);
            }
        }
    }
    if bpp == 32 && (masks.3 == 0 || !any_alpha) {
        rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
    let mut out = Vec::with_capacity(w * h);
    {
        let mut encoder = png::Encoder::new(&mut out, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&rgba)?;
        writer.finish()?;
    }
    Ok(out)
}

pub fn png_to_dib(data: &[u8]) -> Result<Vec<u8>, ImageError> {
    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(data),
        png::Limits {
            bytes: DECODE_LIMIT,
        },
    );
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let size = reader
        .output_buffer_size()
        .ok_or(ImageError::Corrupt("ukuran PNG terlalu besar"))?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf)?;
    let (w, h) = (info.width as usize, info.height as usize);
    if (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(ImageError::Unsupported(format!("{w}×{h} piksel terlalu besar")));
    }
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => {
            return Err(ImageError::Unsupported("PNG berpalet".into()));
        }
    };
    let pixel_bytes = w * h * 4;
    let mut dib = Vec::with_capacity(40 + pixel_bytes);
    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&(w as i32).to_le_bytes());
    dib.extend_from_slice(&(h as i32).to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&BI_RGB.to_le_bytes());
    dib.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    dib.extend_from_slice(&2835i32.to_le_bytes());
    dib.extend_from_slice(&2835i32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    for row in (0..h).rev() {
        let line = &buf[row * info.line_size..row * info.line_size + w * channels];
        for px in line.chunks_exact(channels) {
            let (r, g, b, a) = match channels {
                4 => (px[0], px[1], px[2], px[3]),
                3 => (px[0], px[1], px[2], 255),
                2 => (px[0], px[0], px[0], px[1]),
                _ => (px[0], px[0], px[0], 255),
            };
            dib.extend_from_slice(&[b, g, r, a]);
        }
    }
    Ok(dib)
}
