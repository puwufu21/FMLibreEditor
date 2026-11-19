//! Where the game run by Proton keeps its user data on Linux: in the Wine prefix Steam made for it,
//! `steamapps/compatdata/<app id>/pfx/drive_c/users/steamuser/Documents`, in the Steam library the game is installed
//! in. The graphics packs (faces, logos, flags) are under it, as under the documents folder on Windows.

use std::path::{Path, PathBuf};

/// The documents folder of the Proton prefix in which Football Manager `edition` keeps its user data, if one does.
pub fn documents(edition: u32) -> Option<PathBuf> {
    let game = Path::new("Sports Interactive").join(format!("Football Manager {edition}"));
    libraries().into_iter().find_map(|library| {
        let compatdata = library.join("steamapps").join("compatdata");
        std::fs::read_dir(compatdata).ok()?.flatten().find_map(|prefix| {
            let documents = prefix.path().join("pfx/drive_c/users/steamuser/Documents");
            documents.join(&game).is_dir().then_some(documents)
        })
    })
}

/// The Steam libraries: Steam's own folder, wherever this install of Steam keeps it, and those its
/// `libraryfolders.vdf` lists.
fn libraries() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let roots = [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];
    let mut libraries: Vec<PathBuf> = Vec::new();
    for root in roots.iter().filter(|root| root.is_dir()) {
        let listed = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")).unwrap_or_default();
        for library in std::iter::once(root.clone()).chain(library_paths(&listed)) {
            let library = library.canonicalize().unwrap_or(library);
            if !libraries.contains(&library) {
                libraries.push(library);
            }
        }
    }
    libraries
}

/// The `"path"` values of Steam's `libraryfolders.vdf`.
fn library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| line.trim().strip_prefix("\"path\""))
        .map(|value| PathBuf::from(value.trim().trim_matches('"').replace("\\\\", "\\")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_libraries_steam_lists() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/me/.local/share/Steam\"\n\t}\n\
                   \t\"1\"\n\t{\n\t\t\"path\"\t\t\"/mnt/games/SteamLibrary\"\n\t\t\"label\"\t\t\"\"\n\t}\n}\n";
        assert_eq!(
            library_paths(vdf),
            [PathBuf::from("/home/me/.local/share/Steam"), PathBuf::from("/mnt/games/SteamLibrary")]
        );
    }
}
