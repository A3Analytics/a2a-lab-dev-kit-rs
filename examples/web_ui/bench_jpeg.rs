//! Small baseline JPEG frames for the example bench recording.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::needless_range_loop,
    clippy::unreadable_literal,
    dead_code
)]
//!
//! Each frame is 320 by 192. A bright bar steps across a dark field so playback
//! is visible. Blocks are solid colors, so only the JPEG DC coefficient and an
//! end-of-block marker are encoded.

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
    let mut grid = [[NAVY; BLOCKS_X]; BLOCKS_Y];
    paint_clock(&mut grid, hour, minute, second, step);
    encode(&grid)
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

fn paint_clock(
    grid: &mut [[[u8; 3]; BLOCKS_X]; BLOCKS_Y],
    hour: u32,
    minute: u32,
    second: u32,
    step: u32,
) {
    let digits = [
        hour / 10,
        hour % 10,
        minute / 10,
        minute % 10,
        second / 10,
        second % 10,
    ];
    let mut column = 4_usize;
    for (index, digit) in digits.iter().enumerate() {
        paint_digit(grid, column, 8, *digit);
        column += 5;
        if index == 1 || index == 3 {
            paint_colon(grid, column, 8);
            column += 2;
        }
    }
    let phase = usize::try_from((second.wrapping_add(step)) % 4).unwrap_or(0);
    let mut column = 12_usize;
    for tick in 0..4 {
        let color = if tick <= phase { AMBER } else { [22, 48, 82] };
        fill_color(grid, column, 16, 3, 2, color);
        column += 4;
    }
}

fn paint_digit(grid: &mut [[[u8; 3]; BLOCKS_X]; BLOCKS_Y], column: usize, row: usize, digit: u32) {
    let bits = SEGMENTS[usize::try_from(digit % 10).unwrap_or(0)];
    let on = |segment: u8| bits & (1 << segment) != 0;
    if on(0) {
        fill(grid, column, row, 4, 1);
    }
    if on(1) {
        fill(grid, column, row + 1, 1, 2);
    }
    if on(2) {
        fill(grid, column + 3, row + 1, 1, 2);
    }
    if on(3) {
        fill(grid, column, row + 3, 4, 1);
    }
    if on(4) {
        fill(grid, column, row + 4, 1, 2);
    }
    if on(5) {
        fill(grid, column + 3, row + 4, 1, 2);
    }
    if on(6) {
        fill(grid, column, row + 6, 4, 1);
    }
}

fn paint_colon(grid: &mut [[[u8; 3]; BLOCKS_X]; BLOCKS_Y], column: usize, row: usize) {
    fill(grid, column, row + 2, 1, 1);
    fill(grid, column, row + 5, 1, 1);
}

fn fill(
    grid: &mut [[[u8; 3]; BLOCKS_X]; BLOCKS_Y],
    column: usize,
    row: usize,
    width: usize,
    height: usize,
) {
    fill_color(grid, column, row, width, height, AMBER);
}

fn fill_color(
    grid: &mut [[[u8; 3]; BLOCKS_X]; BLOCKS_Y],
    column: usize,
    row: usize,
    width: usize,
    height: usize,
    color: [u8; 3],
) {
    for y in row..row + height {
        for x in column..column + width {
            if y < BLOCKS_Y && x < BLOCKS_X {
                grid[y][x] = color;
            }
        }
    }
}

const SEGMENTS: [u8; 10] = [
    0b111_0111, 0b010_0100, 0b101_1101, 0b110_1101, 0b010_1110, 0b110_1011, 0b111_1011, 0b100_0100,
    0b111_1111, 0b110_1111,
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
        assert_eq!(clock_parts(3_723_250_000), (1, 2, 3, 1));
        assert_eq!(captured_at(0), "2024-01-01T00:00:00.000000Z");
        assert_eq!(captured_at(4), "2024-01-01T00:00:01.000000Z");
        assert_eq!(captured_at(FRAME_COUNT - 1), "2024-01-01T00:04:59.750000Z");
    }
}
