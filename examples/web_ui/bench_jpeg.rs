//! Small baseline JPEG frames for the example bench recording.
//!
//! Each frame is 320 by 192. A bright bar steps across a dark field so playback
//! is visible. The example clock draws the capture time as pixel numerals. Flat
//! areas encode only a DC coefficient. Glyph blocks also encode AC coefficients.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::needless_range_loop,
    clippy::unreadable_literal,
    dead_code
)]

use std::sync::OnceLock;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 192;

const DC_LUM: [(u8, u16); 12] = [
    (2, 0b00),
    (3, 0b010),
    (3, 0b011),
    (3, 0b100),
    (3, 0b101),
    (3, 0b110),
    (4, 0b1110),
    (5, 0b11110),
    (6, 0b111110),
    (7, 0b1111110),
    (8, 0b11111110),
    (9, 0b111111110),
];

const DC_CHROME: [(u8, u16); 12] = [
    (2, 0b00),
    (2, 0b01),
    (2, 0b10),
    (3, 0b110),
    (4, 0b1110),
    (5, 0b11110),
    (6, 0b111110),
    (7, 0b1111110),
    (8, 0b11111110),
    (9, 0b111111110),
    (10, 0b1111111110),
    (11, 0b11111111110),
];

pub const FRAME_WIDTH: u32 = WIDTH;
pub const FRAME_HEIGHT: u32 = HEIGHT;
pub const FRAME_COUNT: u32 = 5 * 60 * 4;
pub const FRAME_STEP_US: i64 = 250_000;
pub const RETAIN_US: i64 = 120 * 1_000_000;

pub fn clock_parts(micros: i64) -> (u32, u32, u32, u32) {
    let day = micros.rem_euclid(86_400_000_000);
    let step = u32::try_from((day.rem_euclid(1_000_000)) / FRAME_STEP_US).unwrap_or(0);
    let total_seconds = day / 1_000_000;
    let second = u32::try_from(total_seconds.rem_euclid(60)).unwrap_or(0);
    let minute = u32::try_from((total_seconds / 60).rem_euclid(60)).unwrap_or(0);
    let hour = u32::try_from((total_seconds / 3600).rem_euclid(24)).unwrap_or(0);
    (hour, minute, second, step)
}

pub fn captured_at(index: u32) -> String {
    let elapsed = i64::from(index) * FRAME_STEP_US;
    let micros = elapsed.rem_euclid(1_000_000);
    let total_seconds = elapsed.div_euclid(1_000_000);
    let seconds = total_seconds.rem_euclid(60);
    let minutes = (total_seconds / 60).rem_euclid(60);
    let hours = total_seconds / 3600;
    format!("2024-01-01T{hours:02}:{minutes:02}:{seconds:02}.{micros:06}Z")
}

pub fn frame(index: u32) -> Vec<u8> {
    let mut grid = [[NAVY; BLOCKS_X]; BLOCKS_Y];
    let bar = (index % 10) as usize;
    for row in grid.iter_mut().enumerate() {
        let stripe = row.0 % 2 == 0;
        for (column, cell) in row.1.iter_mut().enumerate() {
            *cell = if column == bar {
                AMBER
            } else if stripe {
                [22, 48, 82]
            } else {
                NAVY
            };
        }
    }
    encode(&grid)
}

pub fn clock_frame(hour: u32, minute: u32, second: u32, step: u32) -> Vec<u8> {
    encode_clock(hour, minute, second, step)
}

fn encode(grid: &[[[u8; 3]; BLOCKS_X]; BLOCKS_Y]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(2048);
    bytes.extend_from_slice(&[0xFF, 0xD8]);
    app0(&mut bytes);
    quant_tables(&mut bytes);
    frame_header(&mut bytes);
    huffman_tables(&mut bytes);
    scan(&mut bytes, grid);
    bytes.extend_from_slice(&[0xFF, 0xD9]);
    bytes
}

fn app0(bytes: &mut Vec<u8>) {
    marker(bytes, 0xE0, |body| {
        body.extend_from_slice(b"JFIF\0");
        body.extend_from_slice(&[1, 1, 0, 0, 1, 0, 1, 0, 0]);
    });
}

fn quant_tables(bytes: &mut Vec<u8>) {
    marker(bytes, 0xDB, |body| {
        body.push(0x00);
        body.extend_from_slice(&LUMINANCE_QUANT);
        body.push(0x01);
        body.extend_from_slice(&CHROMINANCE_QUANT);
    });
}

fn frame_header(bytes: &mut Vec<u8>) {
    marker(bytes, 0xC0, |body| {
        body.push(8);
        body.extend_from_slice(&u16::try_from(HEIGHT).unwrap_or(0).to_be_bytes());
        body.extend_from_slice(&u16::try_from(WIDTH).unwrap_or(0).to_be_bytes());
        body.push(3);
        body.extend_from_slice(&[1, 0x11, 0]);
        body.extend_from_slice(&[2, 0x11, 1]);
        body.extend_from_slice(&[3, 0x11, 1]);
    });
}

fn huffman_tables(bytes: &mut Vec<u8>) {
    marker(bytes, 0xC4, |body| {
        dc_table(
            body,
            0x00,
            &[0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0],
        );
        ac_eob(body, 0x10);
        dc_table(
            body,
            0x01,
            &[0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0],
        );
        ac_eob(body, 0x11);
    });
}

fn dc_table(body: &mut Vec<u8>, class_id: u8, counts: &[u8; 16]) {
    body.push(class_id);
    body.extend_from_slice(counts);
    let symbols = usize::from(counts.iter().sum::<u8>());
    body.extend(0..u8::try_from(symbols).unwrap_or(0));
}

fn ac_eob(body: &mut Vec<u8>, class_id: u8) {
    body.push(class_id);
    body.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    body.push(0x00);
}

fn scan(bytes: &mut Vec<u8>, grid: &[[[u8; 3]; BLOCKS_X]; BLOCKS_Y]) {
    marker(bytes, 0xDA, |body| {
        body.push(3);
        body.extend_from_slice(&[1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);
    });
    let mut bits = Bits::default();
    let mut previous = [0_i32; 3];
    for block_y in 0..BLOCKS_Y {
        for block_x in 0..BLOCKS_X {
            let color = grid[block_y][block_x];
            let (y, cb, cr) = ycbcr(color[0], color[1], color[2]);
            previous[0] = write_dc(&mut bits, y, previous[0], &DC_LUM, 16);
            previous[1] = write_dc(&mut bits, cb, previous[1], &DC_CHROME, 17);
            previous[2] = write_dc(&mut bits, cr, previous[2], &DC_CHROME, 17);
        }
    }
    bytes.extend_from_slice(&bits.finish());
}

const BLOCKS_X: usize = (WIDTH / 8) as usize;
const BLOCKS_Y: usize = (HEIGHT / 8) as usize;
const NAVY: [u8; 3] = [12, 28, 52];
const AMBER: [u8; 3] = [244, 176, 44];

const DIGITS: [[u8; 7]; 10] = [
    [
        0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
    ],
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ],
    [
        0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
    ],
    [
        0b10001, 0b10001, 0b10001, 0b11111, 0b00001, 0b00001, 0b00001,
    ],
    [
        0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
    ],
    [
        0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
    ],
];

const GLYPH_SCALE: usize = 4;
const DIGIT_BLOCKS_X: usize = 3;
const DIGIT_BLOCKS_Y: usize = 4;
const CLOCK_X: usize = 10;
const CLOCK_Y: usize = 9;

struct Coeff {
    y: [i16; 64],
    cb: [i16; 64],
    cr: [i16; 64],
}

struct Atlas {
    digits: Vec<Vec<Coeff>>,
    colon: Vec<Coeff>,
    tick: Vec<Coeff>,
}

struct Stamp {
    x: usize,
    y: usize,
    width: usize,
    blocks: &'static [Coeff],
}

fn encode_clock(hour: u32, minute: u32, second: u32, step: u32) -> Vec<u8> {
    let atlas = atlas();
    let digits = [
        hour / 10,
        hour % 10,
        minute / 10,
        minute % 10,
        second / 10,
        second % 10,
    ];
    let mut stamps = Vec::with_capacity(9);
    let mut column = CLOCK_X;
    for (index, digit) in digits.iter().enumerate() {
        stamps.push(Stamp {
            x: column,
            y: CLOCK_Y,
            width: DIGIT_BLOCKS_X,
            blocks: &atlas.digits[usize::try_from(*digit).unwrap_or(0)],
        });
        column += DIGIT_BLOCKS_X;
        if index == 1 || index == 3 {
            stamps.push(Stamp {
                x: column,
                y: CLOCK_Y,
                width: 1,
                blocks: &atlas.colon,
            });
            column += 1;
        }
    }
    let phase = usize::try_from((second.wrapping_add(step)) % 4).unwrap_or(0);
    for tick in 0..=phase {
        stamps.push(Stamp {
            x: 16 + tick * 2,
            y: BLOCKS_Y - 2,
            width: 1,
            blocks: &atlas.tick,
        });
    }
    let mut bytes = Vec::with_capacity(4096);
    bytes.extend_from_slice(&[0xFF, 0xD8]);
    app0(&mut bytes);
    quant_tables(&mut bytes);
    frame_header(&mut bytes);
    clock_huffman_tables(&mut bytes);
    clock_scan(&mut bytes, &stamps);
    bytes.extend_from_slice(&[0xFF, 0xD9]);
    bytes
}

fn atlas() -> &'static Atlas {
    static ATLAS: OnceLock<Atlas> = OnceLock::new();
    ATLAS.get_or_init(|| Atlas {
        digits: (0..10).map(raster_digit).collect(),
        colon: raster_colon(),
        tick: raster_tick(),
    })
}

fn raster_digit(digit: usize) -> Vec<Coeff> {
    let mut pixels = vec![NAVY; 24 * 32];
    let glyph = DIGITS[digit];
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..5 {
            if bits & (1 << (4 - col)) != 0 {
                fill_pixels(
                    &mut pixels,
                    24,
                    2 + col * GLYPH_SCALE,
                    2 + row * GLYPH_SCALE,
                    GLYPH_SCALE,
                    GLYPH_SCALE,
                );
            }
        }
    }
    coeff_tile(&pixels, 24, DIGIT_BLOCKS_X, DIGIT_BLOCKS_Y)
}

fn raster_colon() -> Vec<Coeff> {
    let mut pixels = vec![NAVY; 8 * 32];
    fill_pixels(&mut pixels, 8, 2, 8, GLYPH_SCALE, GLYPH_SCALE);
    fill_pixels(&mut pixels, 8, 2, 20, GLYPH_SCALE, GLYPH_SCALE);
    coeff_tile(&pixels, 8, 1, DIGIT_BLOCKS_Y)
}

fn raster_tick() -> Vec<Coeff> {
    let mut pixels = vec![NAVY; 64];
    fill_pixels(&mut pixels, 8, 1, 2, 6, 3);
    coeff_tile(&pixels, 8, 1, 1)
}

fn fill_pixels(
    pixels: &mut [[u8; 3]],
    stride: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) {
    for row in y..y + height {
        for column in x..x + width {
            pixels[row * stride + column] = AMBER;
        }
    }
}

fn coeff_tile(pixels: &[[u8; 3]], stride: usize, blocks_x: usize, blocks_y: usize) -> Vec<Coeff> {
    let mut blocks = Vec::with_capacity(blocks_x * blocks_y);
    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let mut y = [0_i32; 64];
            let mut cb = [0_i32; 64];
            let mut cr = [0_i32; 64];
            for row in 0..8 {
                for column in 0..8 {
                    let pixel = pixels[(block_y * 8 + row) * stride + block_x * 8 + column];
                    let (yy, ccb, ccr) = ycbcr(pixel[0], pixel[1], pixel[2]);
                    let index = row * 8 + column;
                    y[index] = yy - 128;
                    cb[index] = ccb - 128;
                    cr[index] = ccr - 128;
                }
            }
            blocks.push(Coeff {
                y: quantize(&y, &LUMINANCE_QUANT),
                cb: quantize(&cb, &CHROMINANCE_QUANT),
                cr: quantize(&cr, &CHROMINANCE_QUANT),
            });
        }
    }
    blocks
}

fn quantize(samples: &[i32; 64], quant: &[u8; 64]) -> [i16; 64] {
    let mut out = [0_i16; 64];
    if samples.iter().all(|sample| *sample == samples[0]) {
        out[0] = div_round(8 * samples[0], i32::from(quant[0])).clamp(-2047, 2047) as i16;
        return out;
    }
    let freq = fdct(samples);
    for index in 0..64 {
        let natural = ZIGZAG[index];
        let value = div_round(freq[natural], i32::from(quant[index])).clamp(-2047, 2047);
        out[index] = value as i16;
    }
    out
}

fn fdct(samples: &[i32; 64]) -> [i32; 64] {
    let cos = dct_basis();
    let mut horizontal = [0_f32; 64];
    for y in 0..8 {
        for u in 0..8 {
            let mut sum = 0_f32;
            for x in 0..8 {
                sum += f32::from(i16::try_from(samples[y * 8 + x]).unwrap_or(0)) * cos[u][x];
            }
            let scale = if u == 0 {
                std::f32::consts::FRAC_1_SQRT_2
            } else {
                1.0
            };
            horizontal[y * 8 + u] = sum * scale;
        }
    }
    let mut freq = [0_i32; 64];
    for u in 0..8 {
        for v in 0..8 {
            let mut sum = 0_f32;
            for y in 0..8 {
                sum += horizontal[y * 8 + u] * cos[v][y];
            }
            let scale = if v == 0 {
                std::f32::consts::FRAC_1_SQRT_2
            } else {
                1.0
            };
            freq[v * 8 + u] = (0.25 * sum * scale).round() as i32;
        }
    }
    freq
}

fn dct_basis() -> &'static [[f32; 8]; 8] {
    static BASIS: OnceLock<[[f32; 8]; 8]> = OnceLock::new();
    BASIS.get_or_init(|| {
        let mut table = [[0_f32; 8]; 8];
        for u in 0..8 {
            let u_scale = f32::from(u8::try_from(u).unwrap_or(0));
            for x in 0..8 {
                let column = f32::from(u8::try_from(x).unwrap_or(0));
                table[u][x] = ((2.0 * column + 1.0) * u_scale * std::f32::consts::PI / 16.0).cos();
            }
        }
        table
    })
}

fn clock_huffman_tables(bytes: &mut Vec<u8>) {
    marker(bytes, 0xC4, |body| {
        dc_table(
            body,
            0x00,
            &[0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0],
        );
        dc_table(
            body,
            0x01,
            &[0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0],
        );
        body.push(0x10);
        let mut counts = [0_u8; 16];
        counts[7] = u8::try_from(ac_symbols().len()).unwrap_or(0);
        body.extend_from_slice(&counts);
        body.extend_from_slice(&ac_symbols());
    });
}

fn ac_symbols() -> [u8; 178] {
    let mut symbols = [0_u8; 178];
    let mut index = 1_usize;
    for size in 1..=11_u8 {
        for run in 0..16_u8 {
            symbols[index] = (run << 4) | size;
            index += 1;
        }
    }
    symbols[index] = 0xF0;
    symbols
}

fn ac_codes() -> &'static [u16; 256] {
    static CODES: OnceLock<[u16; 256]> = OnceLock::new();
    CODES.get_or_init(|| {
        let mut codes = [u16::MAX; 256];
        for (code, symbol) in ac_symbols().into_iter().enumerate() {
            codes[usize::from(symbol)] = u16::try_from(code).unwrap_or(u16::MAX);
        }
        codes
    })
}

fn clock_scan(bytes: &mut Vec<u8>, stamps: &[Stamp]) {
    marker(bytes, 0xDA, |body| {
        body.push(3);
        body.extend_from_slice(&[1, 0x00, 2, 0x10, 3, 0x10, 0, 63, 0]);
    });
    let navy = navy_coeff();
    let navy_dc = [
        i32::from(navy.y[0]),
        i32::from(navy.cb[0]),
        i32::from(navy.cr[0]),
    ];
    let mut bits = Bits::default();
    let mut previous = [0_i32; 3];
    let mut flat_run = 0_u32;
    for block_y in 0..BLOCKS_Y {
        for block_x in 0..BLOCKS_X {
            let coeff = stamp_at(stamps, block_x, block_y).unwrap_or(navy);
            if std::ptr::eq(coeff, navy) && previous == navy_dc {
                flat_run += 1;
                continue;
            }
            bits.put_zeros(flat_run * 30);
            flat_run = 0;
            previous[0] = write_coeff(&mut bits, i32::from(coeff.y[0]), previous[0], &DC_LUM);
            write_ac(&mut bits, &coeff.y);
            previous[1] = write_coeff(&mut bits, i32::from(coeff.cb[0]), previous[1], &DC_CHROME);
            write_ac(&mut bits, &coeff.cb);
            previous[2] = write_coeff(&mut bits, i32::from(coeff.cr[0]), previous[2], &DC_CHROME);
            write_ac(&mut bits, &coeff.cr);
        }
    }
    bits.put_zeros(flat_run * 30);
    bytes.extend_from_slice(&bits.finish());
}

fn navy_coeff() -> &'static Coeff {
    static NAVY_COEFF: OnceLock<Coeff> = OnceLock::new();
    NAVY_COEFF.get_or_init(|| {
        let pixels = [NAVY; 64];
        coeff_tile(&pixels, 8, 1, 1).remove(0)
    })
}

fn stamp_at(stamps: &[Stamp], block_x: usize, block_y: usize) -> Option<&Coeff> {
    stamps.iter().find_map(|stamp| {
        if block_y < stamp.y || block_x < stamp.x {
            return None;
        }
        let row = block_y - stamp.y;
        let column = block_x - stamp.x;
        let height = stamp.blocks.len() / stamp.width;
        if row < height && column < stamp.width {
            Some(&stamp.blocks[row * stamp.width + column])
        } else {
            None
        }
    })
}

fn write_coeff(bits: &mut Bits, value: i32, previous: i32, table: &[(u8, u16); 12]) -> i32 {
    let diff = value - previous;
    let class = category(diff);
    let (length, code) = table[usize::from(class)];
    bits.put(u32::from(code), length);
    if class > 0 {
        bits.put(extra_bits(diff, class), class);
    }
    value
}

fn write_ac(bits: &mut Bits, coeffs: &[i16; 64]) {
    let codes = ac_codes();
    let mut run = 0_u8;
    let mut emitted = false;
    for index in 1..64 {
        let value = i32::from(coeffs[index]);
        if value == 0 {
            run += 1;
            continue;
        }
        emitted = true;
        while run >= 16 {
            bits.put(u32::from(codes[0xF0]), 8);
            run -= 16;
        }
        let class = category(value);
        let symbol = usize::from((run << 4) | class);
        bits.put(u32::from(codes[symbol]), 8);
        bits.put(extra_bits(value, class), class);
        run = 0;
    }
    if !emitted || run > 0 {
        bits.put(u32::from(codes[0x00]), 8);
    }
}

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

fn ycbcr(red: u8, green: u8, blue: u8) -> (i32, i32, i32) {
    let red = i32::from(red);
    let green = i32::from(green);
    let blue = i32::from(blue);
    let y = (19595 * red + 38470 * green + 7471 * blue + 32768) >> 16;
    let cb = ((-11059 * red - 21709 * green + 32768 * blue + 32768) >> 16) + 128;
    let cr = ((32768 * red - 27439 * green - 5329 * blue + 32768) >> 16) + 128;
    (y.clamp(0, 255), cb.clamp(0, 255), cr.clamp(0, 255))
}

fn write_dc(
    bits: &mut Bits,
    level: i32,
    previous: i32,
    table: &[(u8, u16); 12],
    quant: i32,
) -> i32 {
    let quantized = div_round(8 * (level - 128), quant);
    let diff = quantized - previous;
    let category = category(diff);
    let (length, code) = table[usize::from(category)];
    bits.put(u32::from(code), length);
    if category > 0 {
        bits.put(extra_bits(diff, category), category);
    }
    bits.put(0b00, 2);
    quantized
}

fn category(diff: i32) -> u8 {
    let magnitude = diff.unsigned_abs();
    if magnitude == 0 {
        0
    } else {
        (32 - magnitude.leading_zeros()) as u8
    }
}

fn extra_bits(diff: i32, category: u8) -> u32 {
    let mask = (1_u32 << category) - 1;
    if diff < 0 {
        (diff as u32).wrapping_sub(1) & mask
    } else {
        diff as u32 & mask
    }
}

fn div_round(value: i32, divisor: i32) -> i32 {
    if value >= 0 {
        (value + divisor / 2) / divisor
    } else {
        -((-value + divisor / 2) / divisor)
    }
}

fn marker(bytes: &mut Vec<u8>, kind: u8, write: impl FnOnce(&mut Vec<u8>)) {
    let mut body = Vec::new();
    write(&mut body);
    bytes.extend_from_slice(&[0xFF, kind]);
    bytes.extend_from_slice(&u16::try_from(body.len() + 2).unwrap_or(2).to_be_bytes());
    bytes.extend_from_slice(&body);
}

#[derive(Default)]
struct Bits {
    data: Vec<u8>,
    acc: u32,
    used: u8,
}

impl Bits {
    fn put_zeros(&mut self, mut count: u32) {
        while self.used > 0 && count > 0 {
            let length = u8::try_from(count.min(u32::from(8 - self.used))).unwrap_or(0);
            self.put(0, length);
            count -= u32::from(length);
        }
        let bytes = usize::try_from(count / 8).unwrap_or(0);
        self.data.resize(self.data.len() + bytes, 0);
        count %= 8;
        if count > 0 {
            self.put(0, u8::try_from(count).unwrap_or(0));
        }
    }

    fn put(&mut self, value: u32, length: u8) {
        self.acc = (self.acc << length) | (value & ((1_u32 << length) - 1));
        self.used += length;
        while self.used >= 8 {
            self.used -= 8;
            let byte = (self.acc >> self.used) as u8;
            self.data.push(byte);
            if byte == 0xFF {
                self.data.push(0x00);
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.used > 0 {
            let pad = (1_u32 << (8 - self.used)) - 1;
            let byte = ((self.acc << (8 - self.used)) | pad) as u8;
            self.data.push(byte);
            if byte == 0xFF {
                self.data.push(0x00);
            }
        }
        self.data
    }
}

const LUMINANCE_QUANT: [u8; 64] = [
    16, 11, 12, 14, 12, 10, 16, 14, 13, 14, 18, 17, 16, 19, 24, 40, 26, 24, 22, 22, 24, 49, 35, 37,
    29, 40, 58, 51, 61, 60, 57, 51, 56, 55, 64, 72, 92, 78, 64, 68, 87, 69, 55, 56, 80, 109, 81,
    87, 95, 98, 103, 104, 103, 62, 77, 113, 121, 112, 100, 120, 92, 101, 103, 99,
];

const CHROMINANCE_QUANT: [u8; 64] = [
    17, 18, 18, 24, 21, 24, 47, 26, 26, 47, 99, 66, 56, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bench_frame_is_a_baseline_jpeg() {
        let bytes = frame(3);
        assert!(bytes.starts_with(&[0xFF, 0xD8]));
        assert!(bytes.ends_with(&[0xFF, 0xD9]));
        let start = bytes
            .windows(2)
            .position(|pair| pair == [0xFF, 0xC0])
            .unwrap();
        let height = u16::from_be_bytes([bytes[start + 5], bytes[start + 6]]);
        let width = u16::from_be_bytes([bytes[start + 7], bytes[start + 8]]);
        assert_eq!(u32::from(width), FRAME_WIDTH);
        assert_eq!(u32::from(height), FRAME_HEIGHT);
        assert_ne!(frame(0), frame(1));
        assert_ne!(clock_frame(1, 2, 3, 0), clock_frame(1, 2, 4, 0));
        assert_ne!(clock_frame(1, 2, 3, 0), clock_frame(1, 2, 3, 3));
        let (y, _, _) = ycbcr(NAVY[0], NAVY[1], NAVY[2]);
        assert_eq!(fdct(&[y - 128; 64])[0], 8 * (y - 128));
        assert_eq!(clock_parts(3_723_250_000), (1, 2, 3, 1));
        assert_eq!(captured_at(0), "2024-01-01T00:00:00.000000Z");
        assert_eq!(captured_at(4), "2024-01-01T00:00:01.000000Z");
        assert_eq!(captured_at(FRAME_COUNT - 1), "2024-01-01T00:04:59.750000Z");
    }
}
