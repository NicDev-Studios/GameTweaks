use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

const LIBRARY_FOLDERS_MAX_BYTES: usize = 16 * 1024 * 1024;
const APP_MANIFEST_MAX_BYTES: usize = 2 * 1024 * 1024;
const VDF_MAX_DEPTH: usize = 64;
const STEAM_APPS_DIRECTORY_NAMES: [&str; 2] = ["steamapps", "SteamApps"];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamGame {
    pub app_id: u32,
    pub name: String,
}

#[derive(Debug)]
pub struct SteamDiscoveryError;

enum LimitedReadError {
    Io,
    Unsafe,
}

enum ManifestRead {
    Game(SteamGame),
    Invalid,
    IoError,
}

struct ManifestScan {
    had_io_error: bool,
}

pub fn discover_installed_games(
    home_dir: Option<&Path>,
) -> Result<Vec<SteamGame>, SteamDiscoveryError> {
    let (roots, source_error) = platform_steam_roots(home_dir);
    discover_from_roots(roots, source_error)
}

fn discover_from_roots(
    roots: Vec<PathBuf>,
    source_error: bool,
) -> Result<Vec<SteamGame>, SteamDiscoveryError> {
    let (roots, root_error) = unique_existing_directories(roots);
    let root_error = root_error || source_error;
    let mut steamapps_directories = Vec::new();
    let mut configuration_error = false;

    for root in &roots {
        let root_steamapps = steamapps_directories_for(root);

        for steamapps in &root_steamapps {
            steamapps_directories.push(steamapps.clone());
            configuration_error |= add_configured_libraries(
                &steamapps.join("libraryfolders.vdf"),
                &mut steamapps_directories,
            );
        }

        configuration_error |= add_configured_libraries(
            &root.join("config").join("libraryfolders.vdf"),
            &mut steamapps_directories,
        );
    }

    let mut games_by_id = HashMap::new();
    let (steamapps_directories, steamapps_error) =
        unique_existing_directories(steamapps_directories);
    let mut scan_error = false;
    for steamapps in &steamapps_directories {
        let scan = collect_manifests(steamapps, &mut games_by_id);
        scan_error |= scan.had_io_error;
    }
    if games_by_id.is_empty()
        && (root_error || steamapps_error || configuration_error || scan_error)
    {
        return Err(SteamDiscoveryError);
    }
    if root_error || steamapps_error || configuration_error || scan_error {
        tracing::warn!("Steam discovery completed with one or more unreadable locations");
    }

    let mut games: Vec<_> = games_by_id.into_values().collect();
    games.sort_by_cached_key(|game| (game.name.to_lowercase(), game.name.clone(), game.app_id));
    Ok(games)
}

fn add_configured_libraries(path: &Path, steamapps_directories: &mut Vec<PathBuf>) -> bool {
    match path.try_exists() {
        Ok(false) => return false,
        Err(_) => {
            tracing::warn!("a Steam library configuration could not be inspected");
            return true;
        }
        Ok(true) => {}
    }
    let raw = match read_limited(path, LIBRARY_FOLDERS_MAX_BYTES) {
        Ok(raw) => raw,
        Err(_) => {
            tracing::warn!("a Steam library configuration could not be read safely");
            return true;
        }
    };
    let Some(document) = VdfParser::parse(&raw) else {
        tracing::warn!("a Steam library configuration is invalid");
        return true;
    };
    let Some(library_folders) = object_value(&document, "libraryfolders") else {
        tracing::warn!("a Steam library configuration has no library list");
        return true;
    };

    let mut configured_libraries = Vec::new();
    let mut configuration_error = false;
    for (index, value) in library_folders {
        if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let Ok(index) = index.parse::<u32>() else {
            configuration_error = true;
            continue;
        };
        let path = match value {
            VdfValue::Text(path) => Some(path.as_str()),
            VdfValue::Object(object) => text_value(object, "path"),
        };
        let Some(path) = path.map(PathBuf::from).filter(|path| path.is_absolute()) else {
            configuration_error = true;
            continue;
        };

        configured_libraries.push((index, path));
    }
    configured_libraries.sort_by_key(|(index, _)| *index);

    for (_, path) in configured_libraries {
        let candidates = steamapps_directories_for(&path);
        let mut available = false;
        for candidate in &candidates {
            match fs::metadata(candidate) {
                Ok(metadata) if metadata.is_dir() => available = true,
                Ok(_) => configuration_error = true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => configuration_error = true,
            }
        }
        configuration_error |= !available;
        steamapps_directories.extend(candidates);
    }
    if configuration_error {
        tracing::warn!("a configured Steam library is invalid or unavailable");
    }
    configuration_error
}

fn collect_manifests(steamapps: &Path, games_by_id: &mut HashMap<u32, SteamGame>) -> ManifestScan {
    let Ok(entries) = fs::read_dir(steamapps) else {
        return ManifestScan { had_io_error: true };
    };

    let mut manifests = Vec::new();
    let mut io_error = false;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                io_error = true;
                continue;
            }
        };
        if let Some(app_id) = manifest_app_id(&entry.file_name()) {
            manifests.push((app_id, entry.path()));
        }
    }
    if io_error {
        tracing::warn!("one or more Steam manifest entries could not be inspected");
    }
    manifests.sort_by_key(|(app_id, _)| *app_id);

    for (file_app_id, path) in manifests {
        match parse_manifest(&path, file_app_id) {
            ManifestRead::Game(game) => {
                games_by_id.entry(game.app_id).or_insert(game);
            }
            ManifestRead::Invalid => {}
            ManifestRead::IoError => io_error = true,
        }
    }
    ManifestScan {
        had_io_error: io_error,
    }
}

fn manifest_app_id(file_name: &OsStr) -> Option<u32> {
    let file_name = file_name.to_str()?;
    let app_id = file_name
        .strip_prefix("appmanifest_")?
        .strip_suffix(".acf")?;
    if app_id.is_empty() || !app_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    app_id.parse().ok()
}

fn parse_manifest(path: &Path, file_app_id: u32) -> ManifestRead {
    let raw = match read_limited(path, APP_MANIFEST_MAX_BYTES) {
        Ok(raw) => raw,
        Err(LimitedReadError::Io) => return ManifestRead::IoError,
        Err(LimitedReadError::Unsafe) => return ManifestRead::Invalid,
    };
    let Some(document) = VdfParser::parse(&raw) else {
        return ManifestRead::Invalid;
    };
    let Some(app_state) = object_value(&document, "appstate") else {
        return ManifestRead::Invalid;
    };
    let Some(app_id) = text_value(app_state, "appid").and_then(|app_id| app_id.parse::<u32>().ok())
    else {
        return ManifestRead::Invalid;
    };
    let Some(name) = text_value(app_state, "name").map(str::trim) else {
        return ManifestRead::Invalid;
    };
    let Some(install_directory) = text_value(app_state, "installdir").map(str::trim) else {
        return ManifestRead::Invalid;
    };

    if app_id == 0 || app_id != file_app_id || name.is_empty() || install_directory.is_empty() {
        return ManifestRead::Invalid;
    }
    let install_directory = Path::new(install_directory);
    if install_directory.is_absolute()
        || install_directory
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return ManifestRead::Invalid;
    }
    let Some(steamapps) = path.parent() else {
        return ManifestRead::Invalid;
    };
    match fs::metadata(steamapps.join("common").join(install_directory)) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return ManifestRead::Invalid,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ManifestRead::Invalid;
        }
        Err(_) => return ManifestRead::IoError,
    }

    ManifestRead::Game(SteamGame {
        app_id,
        name: name.to_owned(),
    })
}

fn read_limited(path: &Path, max_bytes: usize) -> Result<Vec<u8>, LimitedReadError> {
    let metadata = fs::metadata(path).map_err(|_| LimitedReadError::Io)?;
    if !metadata.is_file() || metadata.len() > max_bytes as u64 {
        return Err(LimitedReadError::Unsafe);
    }
    let file = File::open(path).map_err(|_| LimitedReadError::Io)?;
    let mut reader = file.take((max_bytes + 1) as u64);
    let mut raw = Vec::new();
    reader
        .read_to_end(&mut raw)
        .map_err(|_| LimitedReadError::Io)?;
    if raw.len() > max_bytes {
        return Err(LimitedReadError::Unsafe);
    }
    Ok(raw)
}

fn steamapps_directories_for(root: &Path) -> Vec<PathBuf> {
    if root
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name.eq_ignore_ascii_case("steamapps"))
    {
        return vec![root.to_path_buf()];
    }

    STEAM_APPS_DIRECTORY_NAMES
        .iter()
        .map(|name| root.join(name))
        .collect()
}

fn unique_existing_directories(paths: Vec<PathBuf>) -> (Vec<PathBuf>, bool) {
    let mut seen = HashSet::new();
    let mut directories = Vec::new();
    let mut error = false;

    for path in paths.into_iter().filter(|path| path.is_absolute()) {
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {
                let identity = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if seen.insert(identity) {
                    directories.push(path);
                }
            }
            Ok(_) => {}
            Err(io_error) if io_error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => error = true,
        }
    }
    (directories, error)
}

fn platform_steam_roots(_home_dir: Option<&Path>) -> (Vec<PathBuf>, bool) {
    let mut roots = Vec::new();

    #[cfg(target_os = "linux")]
    roots.extend(linux_steam_roots(_home_dir));

    #[cfg(target_os = "macos")]
    if let Some(home_dir) = _home_dir {
        roots.extend(macos_steam_roots(home_dir));
    }

    #[cfg(windows)]
    let source_error = {
        let (windows_roots, source_error) = windows_steam_roots();
        roots.extend(windows_roots);
        source_error
    };

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let source_error = _home_dir.is_none();

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    let source_error = true;

    (roots, source_error)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_steam_roots(home_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(xdg_data_home) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        roots.push(xdg_data_home.join("Steam"));
        roots.push(xdg_data_home.join("steam"));
    }

    if let Some(home_dir) = home_dir {
        roots.extend([
            home_dir.join(".local/share/Steam"),
            home_dir.join(".local/share/steam"),
            home_dir.join(".steam/root"),
            home_dir.join(".steam/steam"),
            home_dir.join(".steam/debian-installation"),
            home_dir.join(".var/app/com.valvesoftware.Steam/data/Steam"),
            home_dir.join(".var/app/com.valvesoftware.Steam/data/steam"),
            home_dir.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
            home_dir.join(".var/app/com.valvesoftware.Steam/.local/share/steam"),
            home_dir.join("snap/steam/common/.local/share/Steam"),
            home_dir.join("snap/steam/common/.local/share/steam"),
        ]);
    }
    roots
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_steam_roots(home_dir: &Path) -> Vec<PathBuf> {
    vec![home_dir.join("Library/Application Support/Steam")]
}

#[cfg(windows)]
fn windows_steam_roots() -> (Vec<PathBuf>, bool) {
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    };
    use winreg::RegKey;

    let mut roots = Vec::new();
    let mut registry_error = false;
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    match current_user.open_subkey(r"Software\Valve\Steam") {
        Ok(steam) => {
            match steam.get_value::<String, _>("SteamPath") {
                Ok(path) => roots.push(PathBuf::from(path)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => registry_error = true,
            }
            match steam.get_value::<String, _>("SteamExe") {
                Ok(executable) => {
                    if let Some(parent) = Path::new(&executable).parent() {
                        roots.push(parent.to_path_buf());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => registry_error = true,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => registry_error = true,
    }

    let local_machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    for flags in [
        KEY_READ | KEY_WOW64_32KEY,
        KEY_READ | KEY_WOW64_64KEY,
        KEY_READ,
    ] {
        match local_machine.open_subkey_with_flags(r"Software\Valve\Steam", flags) {
            Ok(steam) => match steam.get_value::<String, _>("InstallPath") {
                Ok(path) => roots.push(PathBuf::from(path)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => registry_error = true,
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => registry_error = true,
        }
    }

    for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(program_files) = std::env::var_os(variable)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
        {
            roots.push(program_files.join("Steam"));
        }
    }
    roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    (roots, registry_error)
}

type VdfObject = Vec<(String, VdfValue)>;

#[derive(Debug, Eq, PartialEq)]
enum VdfValue {
    Text(String),
    Object(VdfObject),
}

fn object_value<'a>(object: &'a VdfObject, key: &str) -> Option<&'a VdfObject> {
    object.iter().find_map(|(candidate, value)| {
        if candidate.eq_ignore_ascii_case(key) {
            if let VdfValue::Object(value) = value {
                return Some(value);
            }
        }
        None
    })
}

fn text_value<'a>(object: &'a VdfObject, key: &str) -> Option<&'a str> {
    object.iter().find_map(|(candidate, value)| {
        if candidate.eq_ignore_ascii_case(key) {
            if let VdfValue::Text(value) = value {
                return Some(value.as_str());
            }
        }
        None
    })
}

struct VdfParser<'a> {
    raw: &'a [u8],
    position: usize,
}

impl<'a> VdfParser<'a> {
    fn parse(raw: &'a [u8]) -> Option<VdfObject> {
        let raw = raw.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(raw);
        let mut parser = Self { raw, position: 0 };
        let document = parser.parse_object(false, 0)?;
        parser.skip_ignored()?;
        (parser.position == parser.raw.len()).then_some(document)
    }

    fn parse_object(&mut self, nested: bool, depth: usize) -> Option<VdfObject> {
        if depth > VDF_MAX_DEPTH {
            return None;
        }
        let mut object = Vec::new();

        loop {
            self.skip_ignored()?;
            match self.peek() {
                None => return (!nested).then_some(object),
                Some(b'}') if nested => {
                    self.position += 1;
                    return Some(object);
                }
                Some(b'}') => return None,
                _ => {}
            }

            let key = self.parse_token()?;
            self.skip_ignored()?;
            let value = if self.peek() == Some(b'{') {
                self.position += 1;
                VdfValue::Object(self.parse_object(true, depth + 1)?)
            } else {
                VdfValue::Text(self.parse_token()?)
            };
            object.push((key, value));
        }
    }

    fn parse_token(&mut self) -> Option<String> {
        self.skip_ignored()?;
        match self.peek()? {
            b'"' => self.parse_quoted_token(),
            b'{' | b'}' => None,
            _ => self.parse_unquoted_token(),
        }
    }

    fn parse_quoted_token(&mut self) -> Option<String> {
        self.position += 1;
        let mut value = Vec::new();

        while let Some(byte) = self.next() {
            match byte {
                b'"' => return String::from_utf8(value).ok(),
                b'\\' => {
                    let escaped = self.next()?;
                    match escaped {
                        b'"' | b'\\' => value.push(escaped),
                        b'n' => value.push(b'\n'),
                        b'r' => value.push(b'\r'),
                        b't' => value.push(b'\t'),
                        _ => {
                            value.push(b'\\');
                            value.push(escaped);
                        }
                    }
                }
                _ => value.push(byte),
            }
        }
        None
    }

    fn parse_unquoted_token(&mut self) -> Option<String> {
        let start = self.position;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_whitespace() || matches!(byte, b'{' | b'}' | b'"') {
                break;
            }
            self.position += 1;
        }

        if self.position == start {
            return None;
        }
        String::from_utf8(self.raw[start..self.position].to_vec()).ok()
    }

    fn skip_ignored(&mut self) -> Option<()> {
        loop {
            while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
                self.position += 1;
            }

            if self.remaining().starts_with(b"//") {
                self.position += 2;
                while self.peek().is_some_and(|byte| byte != b'\n') {
                    self.position += 1;
                }
                continue;
            }

            if self.remaining().starts_with(b"/*") {
                self.position += 2;
                while self.position < self.raw.len() && !self.remaining().starts_with(b"*/") {
                    self.position += 1;
                }
                if !self.remaining().starts_with(b"*/") {
                    return None;
                }
                self.position += 2;
                continue;
            }

            return Some(());
        }
    }

    fn remaining(&self) -> &'a [u8] {
        &self.raw[self.position..]
    }

    fn peek(&self) -> Option<u8> {
        self.raw.get(self.position).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.position += 1;
        Some(byte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "gametweaks-steam-test-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock must be after the Unix epoch")
                    .as_nanos(),
                NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("test directory should be created");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn vdf_parser_handles_comments_unquoted_tokens_and_escapes() {
        let document = VdfParser::parse(
            br#"// comment
                "Root"
                {
                    unquoted "D:\\Steam Library"
                    "quote" "A \"quoted\" value"
                }
            "#,
        )
        .expect("VDF should parse");
        let root = object_value(&document, "root").expect("root object should exist");

        assert_eq!(text_value(root, "unquoted"), Some(r"D:\Steam Library"));
        assert_eq!(text_value(root, "quote"), Some("A \"quoted\" value"));
    }

    #[test]
    fn library_folders_support_legacy_and_current_shapes() {
        let directory = TestDirectory::new();
        let legacy = directory.0.join("LegacyLibrary");
        let current = directory.0.join("CurrentLibrary");
        fs::create_dir_all(legacy.join("steamapps")).unwrap();
        fs::create_dir_all(current.join("steamapps")).unwrap();
        let library_file = directory.0.join("libraryfolders.vdf");
        fs::write(
            &library_file,
            format!(
                "\"LibraryFolders\" {{ \"1\" \"{}\" \"2\" {{ \"path\" \"{}\" }} }}",
                vdf_path(&legacy),
                vdf_path(&current)
            ),
        )
        .unwrap();

        let mut steamapps = Vec::new();
        assert!(!add_configured_libraries(&library_file, &mut steamapps));

        assert!(steamapps.contains(&legacy.join("steamapps")));
        assert!(steamapps.contains(&current.join("steamapps")));
    }

    #[test]
    fn invalid_numeric_library_entries_are_reported() {
        let directory = TestDirectory::new();
        let library_file = directory.0.join("libraryfolders.vdf");
        fs::write(
            &library_file,
            br#""libraryfolders" { "0" { "label" "missing path" } "metadata" "ignored" }"#,
        )
        .unwrap();

        let mut steamapps = Vec::new();

        assert!(add_configured_libraries(&library_file, &mut steamapps));
        assert!(steamapps.is_empty());
    }

    #[test]
    fn discovery_does_not_report_an_uncertain_empty_result() {
        let directory = TestDirectory::new();
        let root = directory.0.join("Steam");
        fs::create_dir_all(root.join("steamapps")).unwrap();

        assert!(discover_from_roots(vec![root.clone()], true).is_err());
        assert_eq!(discover_from_roots(vec![root], false).unwrap(), Vec::new());
    }

    #[test]
    fn missing_configured_library_does_not_look_like_an_empty_installation() {
        let directory = TestDirectory::new();
        let root = directory.0.join("Steam");
        let steamapps = root.join("steamapps");
        let unavailable = directory.0.join("UnavailableLibrary");
        fs::create_dir_all(&steamapps).unwrap();
        fs::write(
            steamapps.join("libraryfolders.vdf"),
            format!(
                "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
                vdf_path(&root),
                vdf_path(&unavailable)
            ),
        )
        .unwrap();

        assert!(discover_from_roots(vec![root], false).is_err());
    }

    #[test]
    fn discovery_scans_all_libraries_deduplicates_and_sorts_games() {
        let directory = TestDirectory::new();
        let primary = directory.0.join("Steam");
        let external = directory.0.join("ExternalLibrary");
        let primary_steamapps = primary.join("steamapps");
        let external_steamapps = external.join("steamapps");
        fs::create_dir_all(&primary_steamapps).unwrap();
        fs::create_dir_all(&external_steamapps).unwrap();
        fs::create_dir_all(primary_steamapps.join("common/Game20")).unwrap();
        fs::create_dir_all(external_steamapps.join("common/Game10")).unwrap();
        fs::create_dir_all(external_steamapps.join("common/Game20")).unwrap();
        fs::write(
            primary_steamapps.join("libraryfolders.vdf"),
            format!(
                "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
                vdf_path(&primary),
                vdf_path(&external)
            ),
        )
        .unwrap();
        write_manifest(&primary_steamapps, 20, "Zulu");
        write_manifest(&external_steamapps, 10, "alpha");
        write_manifest(&external_steamapps, 20, "Duplicate");
        fs::write(
            external_steamapps.join("appmanifest_30.acf"),
            b"not valid VDF",
        )
        .unwrap();

        let games = discover_from_roots(vec![primary], false).unwrap();

        assert_eq!(
            games,
            vec![
                SteamGame {
                    app_id: 10,
                    name: "alpha".into(),
                },
                SteamGame {
                    app_id: 20,
                    name: "Zulu".into(),
                },
            ]
        );
    }

    #[test]
    fn platform_candidates_cover_supported_unix_installations() {
        let home = Path::new("/home/person");

        assert!(linux_steam_roots(Some(home)).contains(&home.join(".local/share/Steam")));
        assert!(linux_steam_roots(Some(home))
            .contains(&home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam")));
        assert_eq!(
            macos_steam_roots(home),
            vec![home.join("Library/Application Support/Steam")]
        );
    }

    #[test]
    fn parser_rejects_unterminated_block_comments() {
        assert!(VdfParser::parse(br#""root" "value" /* unfinished"#).is_none());
    }

    #[test]
    fn manifest_install_directory_must_stay_below_common() {
        let directory = TestDirectory::new();
        let steamapps = directory.0.join("steamapps");
        fs::create_dir_all(steamapps.join("common")).unwrap();
        let manifest = steamapps.join("appmanifest_1.acf");
        fs::write(
            &manifest,
            br#""AppState" { "appid" "1" "name" "Example" "installdir" "." }"#,
        )
        .unwrap();

        assert!(matches!(
            parse_manifest(&manifest, 1),
            ManifestRead::Invalid
        ));
    }

    fn write_manifest(steamapps: &Path, app_id: u32, name: &str) {
        fs::write(
            steamapps.join(format!("appmanifest_{app_id}.acf")),
            format!(
                "\"AppState\" {{ \"appid\" \"{app_id}\" \"name\" \"{name}\" \"installdir\" \"Game{app_id}\" }}"
            ),
        )
        .unwrap();
    }

    fn vdf_path(path: &Path) -> String {
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    }
}
