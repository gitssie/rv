use vnc::{Rect, VncEvent};

/// CPU-side RGBA framebuffer composed from RFB rects.
#[derive(Debug, Clone, Default)]
pub struct Framebuffer {
    pub width: u16,
    pub height: u16,
    /// Packed RGBA8, row-major.
    pub pixels: Vec<u8>,
    pub generation: u64,
    pub desktop_name: String,
}

impl Framebuffer {
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.pixels = vec![0; width as usize * height as usize * 4];
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn apply(&mut self, event: VncEvent) -> Apply {
        match event {
            VncEvent::SetResolution(screen) => {
                self.resize(screen.width, screen.height);
                Apply::Resized
            }
            VncEvent::RawImage(rect, data) => {
                self.blit_rgba(&rect, &data);
                Apply::Dirty
            }
            VncEvent::Copy(dst, src) => {
                self.copy_rect(dst, src);
                Apply::Dirty
            }
            VncEvent::JpegImage(rect, data) => {
                if self.blit_jpeg(&rect, &data) {
                    Apply::Dirty
                } else {
                    Apply::Ignored
                }
            }
            VncEvent::SetCursor(rect, data) => {
                if rect.width != 0 && rect.height != 0 {
                    self.blit_rgba(&rect, &data);
                    Apply::Dirty
                } else {
                    Apply::Ignored
                }
            }
            VncEvent::Text(text) => Apply::Clipboard(text),
            VncEvent::Bell => Apply::Bell,
            VncEvent::Error(e) => Apply::Error(e),
            VncEvent::SetPixelFormat(_) => Apply::Ignored,
            _ => Apply::Ignored,
        }
    }

    fn blit_rgba(&mut self, rect: &Rect, data: &[u8]) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let w = self.width as usize;
        let h = self.height as usize;
        let rw = rect.width as usize;
        let rh = rect.height as usize;
        let expected = rw.saturating_mul(rh).saturating_mul(4);
        if data.len() < expected || rw == 0 || rh == 0 {
            return;
        }
        for row in 0..rh {
            let dy = rect.y as usize + row;
            if dy >= h {
                break;
            }
            let dx = rect.x as usize;
            if dx >= w {
                break;
            }
            let copy_w = rw.min(w - dx);
            let src_off = row * rw * 4;
            let dst_off = (dy * w + dx) * 4;
            let bytes = copy_w * 4;
            if src_off + bytes <= data.len() && dst_off + bytes <= self.pixels.len() {
                self.pixels[dst_off..dst_off + bytes]
                    .copy_from_slice(&data[src_off..src_off + bytes]);
            }
        }
        self.generation = self.generation.wrapping_add(1);
    }

    fn blit_jpeg(&mut self, rect: &Rect, data: &[u8]) -> bool {
        match image::load_from_memory_with_format(data, image::ImageFormat::Jpeg) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                self.blit_rgba(rect, rgba.as_raw());
                true
            }
            Err(_) => false,
        }
    }

    fn copy_rect(&mut self, dst: Rect, src: Rect) {
        let w = self.width as usize;
        let h = self.height as usize;
        let rw = src.width.min(dst.width) as usize;
        let rh = src.height.min(dst.height) as usize;
        if rw == 0 || rh == 0 {
            return;
        }
        let mut tmp = vec![0u8; rw * rh * 4];
        for row in 0..rh {
            let sy = src.y as usize + row;
            let sx = src.x as usize;
            if sy >= h || sx >= w {
                continue;
            }
            let copy_w = rw.min(w - sx);
            let src_off = (sy * w + sx) * 4;
            let tmp_off = row * rw * 4;
            let bytes = copy_w * 4;
            if src_off + bytes <= self.pixels.len() {
                tmp[tmp_off..tmp_off + bytes]
                    .copy_from_slice(&self.pixels[src_off..src_off + bytes]);
            }
        }
        for row in 0..rh {
            let dy = dst.y as usize + row;
            let dx = dst.x as usize;
            if dy >= h || dx >= w {
                continue;
            }
            let copy_w = rw.min(w - dx);
            let dst_off = (dy * w + dx) * 4;
            let tmp_off = row * rw * 4;
            let bytes = copy_w * 4;
            if dst_off + bytes <= self.pixels.len() {
                self.pixels[dst_off..dst_off + bytes]
                    .copy_from_slice(&tmp[tmp_off..tmp_off + bytes]);
            }
        }
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn thumbnail_png(&self, max_edge: u32) -> Option<Vec<u8>> {
        if self.width == 0 || self.height == 0 || self.pixels.is_empty() {
            return None;
        }
        let img =
            image::RgbaImage::from_raw(self.width as u32, self.height as u32, self.pixels.clone())?;
        let img = image::DynamicImage::ImageRgba8(img);
        let thumb = img.thumbnail(max_edge, max_edge);
        let mut out = Vec::new();
        thumb
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .ok()?;
        Some(out)
    }
}

pub enum Apply {
    Dirty,
    Resized,
    Clipboard(String),
    Bell,
    Error(String),
    Ignored,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blit_and_copy() {
        let mut fb = Framebuffer::default();
        fb.resize(4, 2);
        let mut red = vec![0u8; 4 * 4];
        for px in red.chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }
        fb.blit_rgba(
            &Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
            &red,
        );
        assert_eq!(&fb.pixels[0..4], &[255, 0, 0, 255]);
        fb.copy_rect(
            Rect {
                x: 2,
                y: 0,
                width: 2,
                height: 2,
            },
            Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
        );
        assert_eq!(&fb.pixels[8..12], &[255, 0, 0, 255]);
    }
}
