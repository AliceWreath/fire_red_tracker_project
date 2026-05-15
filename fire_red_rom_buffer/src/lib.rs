use std::sync::OnceLock;

static ROM_BUFFER: OnceLock<Vec<u8>> = OnceLock::new();

pub fn fill_rom(path_to_file: &str) -> Result<(), String> {
    if path_to_file.is_empty() {
        return Err(String::from("Must pass a valid file path."));
    }

    let rom = std::fs::read(path_to_file);
    if rom.is_err() {
        return Err(format!("Unable to open file {}, check the path.\nROM static not initialized!", path_to_file));
    }

    let rom = rom.unwrap();
    fill_static_buffer(rom);

    Ok(())
}

pub fn init_rom(path_to_file: &str) -> Result<(), String> {
    fill_rom(path_to_file)
}

fn fill_static_buffer(buffer: Vec<u8>) -> &'static [u8] {
    ROM_BUFFER.get_or_init(|| buffer)
}

pub fn get_rom() -> &'static [u8] {
    ROM_BUFFER.get().expect("Vector not intialized")
}