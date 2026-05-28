use std::sync::OnceLock;

/// Global immutable ROM buffer.
/// 
/// The ROM data is loaded once during initialization and then shared
/// as a static byte slice for the lifetime of the program.
/// 
/// Internally uses [`OnceLock`] to guarantee thread-safe, one-time
/// initialization.
static ROM_BUFFER: OnceLock<Vec<u8>> = OnceLock::new();

/// Loads a ROM file from disk and initializes the global ROM buffer.
/// 
/// # Arguments
/// 
/// * `path_to_file` - Path to the ROM file.
/// 
/// # Errors
/// 
/// Returns an error if:
/// 
/// - The provided path is empty
/// - The file cannot be opened or read.
/// - The ROM buffer could not be initialized.
/// 
/// # Notes
/// 
/// The ROM buffer is only initialized once. Subsequent calls 
/// will not replace the existing buffer.
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

/// Alias for [`fill_rom`] to provide a more intuitive API for users.
/// 
/// Initializes the global ROM buffer from a ROM file path.
/// 
/// # Arguments
/// 
/// * `path_to_file` - Path to the ROM file.
/// 
/// # Errors
/// 
/// Returns the same errors as [`fill_rom`]
pub fn init_rom(path_to_file: &str) -> Result<(), String> {
    fill_rom(path_to_file)
}

/// Initializes the global ROM buffer if it has not already been set.
/// 
/// # Arguments
/// 
/// * `buffer` - ROM byte buffer to store.
/// 
/// # Returns
/// 
/// A static reference to the stored ROM data.
/// 
/// # Notes
/// 
/// If the buffer has already been initialized, the existing
/// buffer is preserved and returned instead of the new one.
fn fill_static_buffer(buffer: Vec<u8>) -> &'static [u8] {
    ROM_BUFFER.get_or_init(|| buffer)
}

/// Returns a shared reference to the global ROM buffer.
/// 
/// # Panics
/// 
/// Panics if the ROM buffer has not yet been initialized.
/// 
/// # Examples
/// 
/// ```ignore
/// init_rom("firered.gba").unwrap();
/// 
/// let rom = get_rom();
/// println!("ROM size: {} bytes", rom.len());
/// ```
pub fn get_rom() -> &'static [u8] {
    ROM_BUFFER.get().expect("Vector not intialized")
}

/// Returns the ROM buffer if it has been initialized, or `None` otherwise.
pub fn try_get_rom() -> Option<&'static [u8]> {
    ROM_BUFFER.get().map(Vec::as_slice)
}