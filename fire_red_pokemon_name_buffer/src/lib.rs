use std::sync::OnceLock;

/// Global repository containing all loaded pokemon names.
/// 
/// The repo is initialized once at runtime and then shared
/// immutably for the remainder of the program's lifetime.
/// 
/// Internally uses [`OnceLock`] (https://doc.rust-lang.org/std/sync/struct.OnceLock.html)
/// for thread-safe one-time initialization.
static NAME_REPO: OnceLock<Vec<String>> = OnceLock::new();

/// Initializes the global pokemon name repository.
/// 
/// # Arguments
/// 
/// * `names` - Vector containing pokmeon names indexed by species id.
/// 
/// # Notes
/// 
/// The repository can only be initialized once. Any subsequent calls will
/// preserve the existing data and ignore the new input.
/// 
/// # Example
/// 
/// ```ignore
/// fill_name_repo(vec![
/// "_".to_string(),
/// "Bulbasaur".to_string(),
/// "Ivysaur".to_string(),
/// ]);
/// ```
pub fn fill_name_repo(names: Vec<String>) {
    NAME_REPO.get_or_init(|| names);
}

/// Returns the global pokemon name repo.
/// 
/// # Returns
/// 
/// A static slice containing all the loaded pokemon names.
/// 
/// # Panics
/// 
/// Panics if the repo has not yet been initializied.
/// 
/// # Example
/// 
/// ```ignore
/// let names = get_name_repo();
/// println!("{}", names[1]); // Bulbasaur
/// ```
pub fn get_name_repo() -> &'static [String] {
    NAME_REPO.get().expect("Name repo not initialize.")
}