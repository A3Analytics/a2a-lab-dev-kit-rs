//! MJPEG transport padding before decode or still retention.

/// Trims V4L2-style MJPEG transport padding after the JPEG end-of-image marker.
#[must_use]
pub fn trim_mjpeg_transport_padding(bytes: &[u8]) -> &[u8] {
    let Some(end) = jpeg_eoi(bytes) else {
        return bytes;
    };
    &bytes[..end]
}

fn jpeg_eoi(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut index = 2;
    while index + 1 < bytes.len() {
        if bytes[index] != 0xFF {
            return None;
        }
        while index < bytes.len() && bytes[index] == 0xFF {
            index += 1;
        }
        if index >= bytes.len() {
            return None;
        }
        let marker = bytes[index];
        index += 1;
        if marker == 0xD9 {
            return Some(index);
        }
        if marker == 0x00 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        if marker == 0xDA {
            if index + 1 >= bytes.len() {
                return None;
            }
            let len = usize::from(u16::from_be_bytes([bytes[index], bytes[index + 1]]));
            if len < 2 || index.checked_add(len)? > bytes.len() {
                return None;
            }
            return jpeg_scan_eoi(bytes, index + len);
        }
        if index + 1 >= bytes.len() {
            return None;
        }
        let len = usize::from(u16::from_be_bytes([bytes[index], bytes[index + 1]]));
        if len < 2 || index.checked_add(len)? > bytes.len() {
            return None;
        }
        index += len;
    }
    None
}

fn jpeg_scan_eoi(bytes: &[u8], mut index: usize) -> Option<usize> {
    while index + 1 < bytes.len() {
        if bytes[index] != 0xFF {
            index += 1;
            continue;
        }
        let next = bytes[index + 1];
        if next == 0x00 || next == 0xFF {
            index += 1 + usize::from(next == 0x00);
            continue;
        }
        if next == 0xD9 {
            return Some(index + 2);
        }
        if (0xD0..=0xD7).contains(&next) {
            index += 2;
            continue;
        }
        return None;
    }
    None
}
