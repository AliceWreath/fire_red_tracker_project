//! Global Pokémon name repository, loaded once from ROM data at startup.
//!
//! Call [`fill_name_repo`] once with the decoded name table, then use
//! [`get_name_repo`] anywhere to look up names by species index.

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
/// * `names` - Vector containing pokemon names indexed by species id.
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
/// Panics if the repo has not yet been initialized.
///
/// # Example
///
/// ```ignore
/// let names = get_name_repo();
/// println!("{}", names[1]); // Bulbasaur
/// ```
pub fn get_name_repo() -> &'static [String] {
    NAME_REPO.get().expect("Name repo not initialized.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repo is a process-wide OnceLock, so first-fill and re-fill
    /// behaviour must be verified inside a single sequential test.
    #[test]
    fn fill_once_then_subsequent_fills_are_ignored() {
        fill_name_repo(vec!["_".into(), "Bulbasaur".into(), "Ivysaur".into()]);
        let repo = get_name_repo();
        assert_eq!(repo.len(), 3);
        assert_eq!(repo[1], "Bulbasaur");

        // A second fill must preserve the original data.
        fill_name_repo(vec!["overwritten".into()]);
        let repo = get_name_repo();
        assert_eq!(repo.len(), 3);
        assert_eq!(repo[2], "Ivysaur");
    }
}
