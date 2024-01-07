use std::{
    simd::{u32x8, Simd, num::SimdUint},
    sync::Arc,
};

use async_trait::async_trait;
use breakwater_core::{framebuffer::FrameBuffer, HELP_TEXT};
use snafu::ResultExt;
use tokio::io::AsyncWriteExt;

use crate::{Parser, ParserError};

const PARSER_LOOKAHEAD: usize = "PX 1234 1234 rrggbbaa\n".len(); // Longest possible command

pub struct SimpleParser {
    connection_x_offset: usize,
    connection_y_offset: usize,
    fb: Arc<FrameBuffer>,
}

impl SimpleParser {
    pub fn new(fb: Arc<FrameBuffer>) -> SimpleParser {
        SimpleParser {
            connection_x_offset: 0,
            connection_y_offset: 0,
            fb,
        }
    }

    #[inline]
    async fn handle_pixel(&self, buffer: &[u8], mut idx: usize, stream: &mut (impl AsyncWriteExt + Send + Unpin)) -> Result<usize, ParserError> {
        let previous = idx;
        idx += 3;

        let (mut x, mut y, present) = parse_pixel_coordinates(buffer.as_ptr(), &mut idx);

        if present {
            x += self.connection_x_offset;
            y += self.connection_y_offset;

            // Separator between coordinates and color
            if unsafe { *buffer.get_unchecked(idx) } == b' ' {
                idx += 1;

                // TODO: Determine what clients use more: RGB, RGBA or gg variant.
                // If RGBA is used more often move the RGB code below the RGBA code

                // Must be followed by 6 bytes RGB and newline or ...
                if unsafe { *buffer.get_unchecked(idx + 6) } == b'\n' {
                    idx += 7;
                    self.handle_rgb(idx, buffer, x, y);
                }

                // ... or must be followed by 8 bytes RGBA and newline
                else if unsafe { *buffer.get_unchecked(idx + 8) } == b'\n' {
                    idx += 9;
                    self.handle_rgba(idx, buffer, x, y);
                }

                // ... for the efficient/lazy clients
                else if unsafe { *buffer.get_unchecked(idx + 2) } == b'\n' {
                    idx += 3;
                    self.handle_gray(idx, buffer, x, y);
                } else {
                    idx = previous
                }
            }

            // End of command to read Pixel value
            else if unsafe { *buffer.get_unchecked(idx) } == b'\n' {
                idx += 1;
                self.handle_get_pixel(stream, x, y).await?;
            } else {
                idx = previous
            }
        } else {
            idx = previous
        }
        Ok(idx)
    }

    #[inline]
    fn handle_offset(&mut self, idx: &mut usize, buffer: &[u8]) {
        let (x, y, present) = parse_pixel_coordinates(buffer.as_ptr(), idx);

        // End of command to set offset
        if present && unsafe { *buffer.get_unchecked(*idx) } == b'\n' {
            self.connection_x_offset = x;
            self.connection_y_offset = y;
        }
    }

    #[inline]
    async fn handle_size(&self, stream: &mut (impl AsyncWriteExt + Send + Unpin)) -> Result<(), ParserError> {
        stream
            .write_all(format!("SIZE {} {}\n", self.fb.get_width(), self.fb.get_height()).as_bytes())
            .await
            .context(crate::WriteToTcpSocketSnafu)?;
        Ok(())
    }

    #[inline]
    async fn handle_help(&self, stream: &mut (impl AsyncWriteExt + Send + Unpin)) -> Result<(), ParserError> {
        stream
            .write_all(HELP_TEXT)
            .await
            .context(crate::WriteToTcpSocketSnafu)?;
        Ok(())
    }

    #[inline]
    fn handle_rgb(&self, idx: usize, buffer: &[u8], x: usize, y: usize) {
        let rgba: u32 = simd_unhex(unsafe { buffer.as_ptr().add(idx - 7) });

        self.fb.set(x, y, rgba & 0x00ff_ffff);
    }

    #[cfg(not(feature = "alpha"))]
    #[inline]
    fn handle_rgba(&self, idx: usize, buffer: &[u8], x: usize, y: usize) {
        let rgba: u32 = simd_unhex(unsafe { buffer.as_ptr().add(idx - 9) });

        self.fb.set(x, y, rgba & 0x00ff_ffff);
    }

    #[cfg(feature = "alpha")]
    #[inline]
    fn handle_rgba(&self, idx: usize, buffer: &[u8], x: usize, y: usize) {
        let rgba: u32 = simd_unhex(unsafe { buffer.as_ptr().add(idx - 9) });

        let alpha = (rgba >> 24) & 0xff;

        if alpha == 0 || x >= self.fb.get_width() || y >= self.fb.get_height() {
            return
        }

        let alpha_comp = 0xff - alpha;
        let current = self.fb.get_unchecked(x, y);
        let r = (rgba >> 16) & 0xff;
        let g = (rgba >> 8) & 0xff;
        let b = rgba & 0xff;

        let r: u32 = (((current >> 24) & 0xff) * alpha_comp + r * alpha) / 0xff;
        let g: u32 = (((current >> 16) & 0xff) * alpha_comp + g * alpha) / 0xff;
        let b: u32 = (((current >> 8) & 0xff) * alpha_comp + b * alpha) / 0xff;

        self.fb.set(x, y, r << 16 | g << 8 | b);
    }

    #[inline]
    fn handle_gray(&self, idx: usize, buffer: &[u8], x: usize, y: usize) {
        // FIXME: Read that two bytes directly instead of going through the whole SIMD vector setup.
        // Or - as an alternative - still do the SIMD part but only load two bytes.
        let base: u32 =
            simd_unhex(unsafe { buffer.as_ptr().add(idx - 3) }) & 0xff;

        let rgba: u32 = base << 16 | base << 8 | base;

        self.fb.set(x, y, rgba);
    }

    #[inline]
    async fn handle_get_pixel(&self, stream: &mut(impl AsyncWriteExt + Send + Unpin), x: usize, y: usize) -> Result<(), ParserError> {
        if let Some(rgb) = self.fb.get(x, y) {
            stream
                .write_all(
                    format!(
                        "PX {} {} {:06x}\n",
                        // We don't want to return the actual (absolute) coordinates, the client should also get the result offseted
                        x - self.connection_x_offset,
                        y - self.connection_y_offset,
                        rgb.to_be() >> 8
                    )
                        .as_bytes(),
                )
                .await
                .context(crate::WriteToTcpSocketSnafu)?;
        }
        Ok(())
    }
}

#[async_trait]
impl Parser for SimpleParser {
    async fn parse(
        &mut self,
        buffer: &[u8],
        mut stream: impl AsyncWriteExt + Send + Unpin,
    ) -> Result<usize, ParserError> {
        let mut i = 0; // We can't use a for loop here because Rust don't lets use skip characters by incrementing i
        let loop_end = buffer.len().saturating_sub(PARSER_LOOKAHEAD); // Let's extract the .len() call and the subtraction into it's own variable so we only compute it once

        while i < loop_end {
            let current_command =
                unsafe { (buffer.as_ptr().add(i) as *const u64).read_unaligned() };
            if current_command & 0x00ff_ffff == string_to_number(b"PX \0\0\0\0\0") {
                i = self.handle_pixel(buffer, i, &mut stream).await?;
            } else if current_command & 0x00ff_ffff_ffff_ffff == string_to_number(b"OFFSET \0\0") {
                i += 7;
                self.handle_offset(&mut i, buffer);
            } else if current_command & 0xffff_ffff == string_to_number(b"SIZE\0\0\0\0") {
                i += 4;
                self.handle_size(&mut stream).await?;
            } else if current_command & 0xffff_ffff == string_to_number(b"HELP\0\0\0\0") {
                i += 4;
                self.handle_help(&mut stream).await?;
            } else {
                i += 1;
            }
        }

        Ok(i - 1)
    }

    fn parser_lookahead() -> usize {
        PARSER_LOOKAHEAD
    }
}

#[inline]
const fn string_to_number(input: &[u8]) -> u64 {
    (input[7] as u64) << 56
        | (input[6] as u64) << 48
        | (input[5] as u64) << 40
        | (input[4] as u64) << 32
        | (input[3] as u64) << 24
        | (input[2] as u64) << 16
        | (input[1] as u64) << 8
        | (input[0] as u64)
}

const SHIFT_PATTERN: Simd<u32, 8> = u32x8::from_array([4, 0, 12, 8, 20, 16, 28, 24]);
const SIMD_6: Simd<u32, 8> = u32x8::from_array([6; 8]);
const SIMD_F: Simd<u32, 8> = u32x8::from_array([0xf; 8]);
const SIMD_9: Simd<u32, 8> = u32x8::from_array([9; 8]);

/// Parse a slice of 8 characters into a single u32 number
/// is undefined behavior for invalid characters
#[inline(always)]
fn simd_unhex(value: *const u8) -> u32 {
    // Feel free to find a better, but fast, way, to cast all integers as u32
    let input = unsafe {
        u32x8::from_array([
            *value as u32,
            *value.add(1) as u32,
            *value.add(2) as u32,
            *value.add(3) as u32,
            *value.add(4) as u32,
            *value.add(5) as u32,
            *value.add(6) as u32,
            *value.add(7) as u32,
        ])
    };

    // Heavily inspired by https://github.com/nervosnetwork/faster-hex/blob/a4c06b387ddeeea311c9e84a3adcaf01015cf40e/src/decode.rs#L80
    let sr6 = input >> SIMD_6;
    let and15 = input & SIMD_F;
    let mul = sr6 * SIMD_9;
    let hexed = and15 + mul;
    let shifted = hexed << SHIFT_PATTERN;
    shifted.reduce_or()
}

#[inline(always)]
fn parse_coordinate(buffer: *const u8, current_index: &mut usize) -> (usize, bool) {
    let digits = unsafe { (buffer.add(*current_index) as *const usize).read_unaligned() };

    let mut result = 0;
    let mut visited = false;
    // The compiler will unroll this loop, but this way, it is more maintainable
    for pos in 0..4 {
        let digit = (digits >> (pos * 8)) & 0xff;
        if digit >= b'0' as usize && digit <= b'9' as usize {
            result = 10 * result + digit - b'0' as usize;
            *current_index += 1;
            visited = true;
        } else {
            break;
        }
    }

    (result, visited)
}

#[inline(always)]
fn parse_pixel_coordinates(buffer: *const u8, current_index: &mut usize) -> (usize, usize, bool) {
    let (x, x_visited) = parse_coordinate(buffer, current_index);
    *current_index += 1;
    let (y, y_visited) = parse_coordinate(buffer, current_index);
    (x, y, x_visited && y_visited)
}
