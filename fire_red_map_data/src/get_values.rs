pub fn get_bytes(buffer: &[&str]) -> Vec<u8> {
    let bytes: Vec<u8> = buffer[..]
        .iter()
        .filter_map(|h| u8::from_str_radix(h, 16).ok())
        .collect();
    bytes
}

pub fn get_u32(buffer: &[&str]) -> u32 {
    let bytes = get_bytes(buffer);
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

pub fn get_i32(buffer: &[&str]) -> i32 {
    let bytes = get_bytes(buffer);
    i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

pub fn get_u16(buffer: &[&str]) -> u16 {
    let bytes = get_bytes(buffer);
    u16::from_le_bytes([bytes[0], bytes[1]])
}

pub fn get_i16(buffer: &[&str]) -> i16 {
    let bytes = get_bytes(buffer);
    i16::from_le_bytes([bytes[0], bytes[1]])
}

pub fn get_u8(buffer: &[&str]) -> u8 {
    let bytes = get_bytes(buffer);
    u8::from_le_bytes([bytes[0]])
}

pub fn read_u8(bytes: &[u8], offset: usize) -> u8 {
    u8::from_le_bytes([bytes[offset]])
}

pub fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

pub fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

pub fn read_i16(bytes: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

pub fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
