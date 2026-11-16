//! Content hashes (BLAKE3) and perceptual hashes (pHash + dHash).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::Result;
use image::imageops::FilterType;
use image::DynamicImage;

pub const PARTIAL_CHUNK: u64 = 64 * 1024;

/// Hash of the first and last 64 KB plus the size. Cheap pre-filter before a full read.
/// Only meaningful for files larger than `2 * PARTIAL_CHUNK`.
pub fn partial_hash(path: &Path, size: u64) -> Result<[u8; 32]> {
    let mut f = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; PARTIAL_CHUNK as usize];
    f.read_exact(&mut buf)?;
    hasher.update(&buf);
    f.seek(SeekFrom::Start(size - PARTIAL_CHUNK))?;
    f.read_exact(&mut buf)?;
    hasher.update(&buf);
    hasher.update(&size.to_le_bytes());
    Ok(*hasher.finalize().as_bytes())
}

pub fn full_hash(path: &Path) -> Result<[u8; 32]> {
    let mut f = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(*hasher.finalize().as_bytes())
}

/// 64-bit DCT perceptual hash: 32x32 greyscale, low-frequency 8x8 block vs its median.
pub fn phash(img: &DynamicImage) -> u64 {
    const N: usize = 32;
    let small = img
        .resize_exact(N as u32, N as u32, FilterType::Triangle)
        .to_luma8();
    let px: Vec<f64> = small.as_raw().iter().map(|&v| v as f64).collect();

    // cos table for the first 8 frequencies only
    let mut cos = [[0f64; N]; 8];
    for (u, row) in cos.iter_mut().enumerate() {
        for (x, c) in row.iter_mut().enumerate() {
            *c = ((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI / (2.0 * N as f64)).cos();
        }
    }
    // rows, then columns
    let mut tmp = [[0f64; 8]; N]; // tmp[y][u]
    for y in 0..N {
        for u in 0..8 {
            let mut s = 0.0;
            for x in 0..N {
                s += px[y * N + x] * cos[u][x];
            }
            tmp[y][u] = s;
        }
    }
    let mut coef = [0f64; 64]; // coef[v*8+u]
    for v in 0..8 {
        for u in 0..8 {
            let mut s = 0.0;
            for (y, row) in tmp.iter().enumerate() {
                s += row[u] * cos[v][y];
            }
            coef[v * 8 + u] = s;
        }
    }
    // median without the DC term, which only carries overall brightness
    let mut sorted: Vec<f64> = coef[1..].to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = sorted[sorted.len() / 2];
    let mut bits = 0u64;
    for (i, c) in coef.iter().enumerate().skip(1) {
        if *c > median {
            bits |= 1 << i;
        }
    }
    bits
}

/// 64-bit gradient hash: 9x8 greyscale, each bit = "pixel brighter than its right neighbour".
pub fn dhash(img: &DynamicImage) -> u64 {
    let small = img.resize_exact(9, 8, FilterType::Triangle).to_luma8();
    let mut bits = 0u64;
    for y in 0..8u32 {
        for x in 0..8u32 {
            if small.get_pixel(x, y)[0] > small.get_pixel(x + 1, y)[0] {
                bits |= 1 << (y * 8 + x);
            }
        }
    }
    bits
}

/// Both hashes must agree for two photos to count as close.
#[inline]
pub fn distance(p1: u64, d1: u64, p2: u64, d2: u64) -> u32 {
    (p1 ^ p2).count_ones().max((d1 ^ d2).count_ones())
}
