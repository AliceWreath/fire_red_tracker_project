fn get_bytes(buffer: &[&str]) -> Vec<u8> {
        let bytes: Vec<u8> = buffer[..]
            .iter()
            .filter_map(|h| u8::from_str_radix(h, 16).ok())
            .collect();
        bytes
    }

pub fn get_n_bytes(n: usize, buffer: &[&str]) -> Vec<u8> {
    if buffer.len() < n {
        return vec![0; n];
    }
    
    let byte_list = get_bytes(&buffer[..n]);

    byte_list
}

pub fn get_u32(buffer: &[&str]) -> u32 {
    if buffer.len() < 4 {
        return 0;
    }
    let bytes: Vec<u8> = buffer.iter()
        .filter_map(|h| u8::from_str_radix(h, 16).ok())
        .collect();

    bytes.get(..4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes).unwrap_or(0)
}

pub fn get_i32(buffer: &[&str]) -> i32 {
    if buffer.len() < 4 {
        return 0;
    }
    let bytes: Vec<u8> = buffer.iter()
        .filter_map(|h| u8::from_str_radix(h, 16).ok())
        .collect();

    bytes.get(..4)
        .and_then(|b| b.try_into().ok())
        .map(i32::from_le_bytes).unwrap_or(0)
}

pub fn get_u16(buffer: &[&str]) -> u16 {
    if buffer.len() < 2{
        return 0;
    }
    let bytes: Vec<u8> = buffer.iter()
        .filter_map(|h| u8::from_str_radix(h, 16).ok())
        .collect();

    bytes.get(..2)
        .and_then(|b| b.try_into().ok())
        .map(u16::from_le_bytes).unwrap_or(0)
}

pub fn get_i16(buffer: &[&str]) -> i16 {
    if buffer.len() < 2{
        return 0;
    }
    let bytes: Vec<u8> = buffer.iter()
        .filter_map(|h| u8::from_str_radix(h, 16).ok())
        .collect();

    bytes.get(..2)
        .and_then(|b| b.try_into().ok())
        .map(i16::from_le_bytes).unwrap_or(0)
}

pub fn get_u8(buffer: &[&str]) -> u8 {
    if buffer.len() < 1 {
        return 0;
    }
    let bytes = get_bytes(buffer);
    if bytes.is_empty() {
        return 0;
    }
    u8::from_le_bytes([bytes[0]])
}

pub fn read_u8(bytes: &[u8], offset: usize) -> u8 {
    if bytes.is_empty() {
        return 0;
    }
    u8::from_le_bytes([bytes[offset]]).try_into().unwrap()
}

pub fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    if bytes.len() < offset + 4 {
        return 0;
    }
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

pub fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    if bytes.len() < offset + 2 {
        return 0;
    }
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

pub fn read_i16(bytes: &[u8], offset: usize) -> i16 {
    if bytes.len() < offset + 2 {
        return 0;
    }
    i16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

pub fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    if bytes.len() < offset + 4 {
        return 0;
    }
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

pub fn read_u8_raw(bytes: &[u8], offset: usize) -> u8 {
    if bytes.is_empty() {
        return 0;
    }
    u8::from_be_bytes([bytes[offset]]).try_into().unwrap()
}

pub fn read_u32_raw(bytes: &[u8], offset: usize) -> u32 {
    if bytes.len() < offset + 4 {
        return 0;
    }
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

pub fn read_u16_raw(bytes: &[u8], offset: usize) -> u16 {
    if bytes.len() < offset + 2 {
        return 0;
    }
    u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

pub fn read_i16_raw(bytes: &[u8], offset: usize) -> i16 {
    if bytes.len() < offset + 2 {
        return 0;
    }
    i16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

pub fn read_i32_raw(bytes: &[u8], offset: usize) -> i32 {
    if bytes.len() < offset + 4 {
        return 0;
    }
    i32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}