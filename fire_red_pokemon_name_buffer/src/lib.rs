use std::sync::OnceLock;

static NAME_REPO: OnceLock<Vec<String>> = OnceLock::new();

pub fn fill_name_repo(names: Vec<String>) {
    NAME_REPO.get_or_init(|| names);
}

pub fn get_name_repo() -> &'static [String] {
    NAME_REPO.get().expect("Name repo not initialize.")
}