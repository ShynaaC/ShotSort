use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, String>;
const SETTLE_TIME: Duration = Duration::from_secs(2);

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub source_dir: Option<PathBuf>,
    pub storage_dir: Option<PathBuf>,
    #[serde(default)]
    pub managed_storage: bool,
    pub active_session_id: Option<String>,
    pub sessions: Vec<Session>,
    #[serde(default)]
    pending_delete: Option<Session>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub name: String,
    pub folder: PathBuf,
    pub created_at: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Screenshot {
    pub name: String,
    pub bytes: u64,
    pub modified_at: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    #[serde(flatten)]
    pub session: Session,
    pub count: usize,
    pub bytes: u64,
    pub storage_file_count: usize,
    pub storage_bytes: u64,
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub source_dir: Option<PathBuf>,
    pub storage_dir: Option<PathBuf>,
    pub managed_storage: bool,
    pub default_storage_dir: PathBuf,
    pub active_session_id: Option<String>,
    pub monitoring: bool,
    pub pending_count: usize,
    pub sessions: Vec<SessionView>,
    pub screenshots: Vec<Screenshot>,
    pub last_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletionPreview {
    pub id: String,
    pub name: String,
    pub folder: PathBuf,
    pub file_count: usize,
    pub bytes: u64,
    pub is_active: bool,
    pub folder_missing: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    bytes: u64,
    modified: SystemTime,
}

struct Pending {
    session_id: String,
    stamp: Stamp,
    stable_since: Instant,
    attempts: u8,
}

pub struct Storage {
    pub config: Config,
    config_path: PathBuf,
    pub monitoring: bool,
    seen: HashSet<PathBuf>,
    pending: HashMap<PathBuf, Pending>,
    pub last_error: Option<String>,
}

fn millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn stamp(path: &Path) -> io::Result<Stamp> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::other("Not a regular file"));
    }
    Ok(Stamp {
        bytes: metadata.len(),
        modified: metadata.modified()?,
    })
}

fn is_image(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp"
        )
    })
}

fn scan(folder: &Path) -> Result<Vec<(PathBuf, Stamp)>> {
    let entries =
        fs::read_dir(folder).map_err(|e| format!("Cannot read {}: {e}", folder.display()))?;
    let mut images = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("Cannot read a folder entry: {e}"))?;
        let path = entry.path();
        if is_image(&path) {
            match stamp(&path) {
                Ok(metadata) => images.push((path, metadata)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(_) if entry.file_type().is_ok_and(|t| !t.is_file()) => {}
                Err(e) => return Err(format!("Cannot inspect {}: {e}", path.display())),
            }
        }
    }
    Ok(images)
}

fn canonical_dir(value: &str) -> Result<PathBuf> {
    let path = Path::new(value.trim());
    if !path.is_absolute() || !path.is_dir() {
        return Err("Choose an existing folder using an absolute path.".into());
    }
    fs::canonicalize(path).map_err(|e| format!("Cannot access that folder: {e}"))
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

fn write_config(path: &Path, config: &Config) -> Result<()> {
    let parent = path.parent().ok_or("Invalid settings path")?;
    fs::create_dir_all(parent).map_err(|e| format!("Cannot create settings folder: {e}"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
    temp.write_all(&bytes)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| format!("Cannot save settings: {e}"))?;
    temp.persist(path)
        .map_err(|e| format!("Cannot replace settings: {e}"))?;
    Ok(())
}

fn has_valid_session_folder_name(session: &Session) -> bool {
    let folder_name = session
        .folder
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let expected_suffix = format!("-{}", session.id);
    session.id.chars().all(|c| c.is_ascii_digit())
        && folder_name.starts_with("session-")
        && folder_name.ends_with(&expected_suffix)
}

fn validate_session_folder(session: &Session) -> Result<()> {
    if !has_valid_session_folder_name(session) {
        return Err("ShotSort will only delete folders it created for sessions.".into());
    }
    let metadata = fs::symlink_metadata(&session.folder)
        .map_err(|e| format!("Session folder is unavailable: {e}"))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("Session folder is not a regular directory. Nothing was deleted.".into());
    }
    let resolved = fs::canonicalize(&session.folder)
        .map_err(|e| format!("Cannot verify the session folder: {e}"))?;
    if resolved != session.folder {
        return Err("Session folder moved or was replaced. Nothing was deleted.".into());
    }
    Ok(())
}

fn path_is_missing(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(format!("Cannot inspect {}: {error}", path.display())),
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if let (Ok(a), Ok(b)) = (fs::canonicalize(a), fs::canonicalize(b)) {
        return a == b;
    }
    #[cfg(windows)]
    {
        fn key(path: &Path) -> String {
            let text = path.to_string_lossy();
            text.strip_prefix(r"\\?\UNC\")
                .map(|rest| format!(r"\\{rest}"))
                .or_else(|| text.strip_prefix(r"\\?\").map(str::to_owned))
                .unwrap_or_else(|| text.into_owned())
                .to_lowercase()
        }
        return key(a) == key(b);
    }
    #[cfg(not(windows))]
    false
}

fn recursive_size(root: &Path) -> Result<(usize, u64)> {
    let mut folders = vec![root.to_path_buf()];
    let mut count = 0;
    let mut bytes = 0u64;
    while let Some(folder) = folders.pop() {
        let entries = fs::read_dir(&folder)
            .map_err(|e| format!("Cannot inspect {}: {e}", folder.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("Cannot inspect a session item: {e}"))?;
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|e| format!("Cannot inspect {}: {e}", entry.path().display()))?;
            if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
                folders.push(entry.path());
            } else {
                count += 1;
                bytes = bytes.saturating_add(metadata.len());
            }
        }
    }
    Ok((count, bytes))
}

impl Storage {
    pub fn load(config_path: PathBuf) -> Result<Self> {
        let mut config = match fs::read(&config_path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                format!(
                    "Saved ShotSort settings are unreadable; they have not been overwritten: {e}"
                )
            })?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(format!("Cannot load saved settings: {e}")),
        };
        if let Some(pending) = config.pending_delete.clone() {
            if !pending.folder.exists() {
                config.sessions.retain(|session| session.id != pending.id);
                if config.active_session_id.as_deref() == Some(pending.id.as_str()) {
                    config.active_session_id = None;
                }
            }
            config.pending_delete = None;
            write_config(&config_path, &config)?;
        }
        // Restoring session selection, but never resume file moves without the user.
        Ok(Self {
            config,
            config_path,
            monitoring: false,
            seen: HashSet::new(),
            pending: HashMap::new(),
            last_error: None,
        })
    }

    fn save(&self, config: &Config) -> Result<()> {
        write_config(&self.config_path, config)
    }

    fn default_storage_dir(&self) -> PathBuf {
        self.config_path.with_file_name("sessions")
    }

    pub fn configure(&mut self, source: &str, destination: &str) -> Result<()> {
        if self.monitoring {
            return Err("Pause the session before changing folders.".into());
        }
        let source = canonical_dir(source)?;
        let managed_storage = destination.trim().is_empty();
        let destination = if managed_storage {
            let folder = self.default_storage_dir();
            fs::create_dir_all(&folder)
                .map_err(|e| format!("Cannot create ShotSort's session storage: {e}"))?;
            fs::canonicalize(&folder)
                .map_err(|e| format!("Cannot access ShotSort's session storage: {e}"))?
        } else {
            canonical_dir(destination)?
        };
        if overlaps(&source, &destination) {
            return Err(
                "Choose separate screenshot and storage folders; neither may contain the other."
                    .into(),
            );
        }
        for session in &self.config.sessions {
            let folder =
                fs::canonicalize(&session.folder).unwrap_or_else(|_| session.folder.clone());
            if overlaps(&source, &folder) {
                return Err(
                    "The screenshot source must be separate from every existing session folder."
                        .into(),
                );
            }
        }
        // Probe the chosen destination without leaving a file behind.
        tempfile::NamedTempFile::new_in(&destination)
            .map_err(|e| format!("Storage folder is not writable: {e}"))?;
        scan(&source)?;
        let mut next = self.config.clone();
        next.source_dir = Some(source);
        next.storage_dir = Some(destination);
        next.managed_storage = managed_storage;
        self.save(&next)?;
        self.config = next;
        self.last_error = None;
        Ok(())
    }

    pub fn create_quick_session(&mut self) -> Result<String> {
        let mut number = self.config.sessions.len() + 1;
        loop {
            let name = format!("Quick session {number}");
            if !self
                .config
                .sessions
                .iter()
                .any(|session| session.name == name)
            {
                return self.create_session(&name);
            }
            number += 1;
        }
    }

    pub fn create_session(&mut self, name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err("Give the session a name between 1 and 80 characters.".into());
        }
        let root = self
            .config
            .storage_dir
            .as_ref()
            .ok_or("Set up your storage folder first.")?;
        if fs::canonicalize(root).ok().as_ref() != Some(root) {
            return Err(
                "Your storage folder has moved or is unavailable. Choose it again in Folder setup."
                    .into(),
            );
        }
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_string();
        let slug: String = name
            .chars()
            .take(32)
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        let folder = root.join(format!("session-{slug}-{id}"));
        fs::create_dir(&folder).map_err(|e| format!("Cannot create session folder: {e}"))?;
        let session = Session {
            id: id.clone(),
            name: name.into(),
            folder: folder.clone(),
            created_at: millis(SystemTime::now()),
        };
        let mut next = self.config.clone();
        next.sessions.insert(0, session);
        if let Err(e) = self.save(&next) {
            let _ = fs::remove_dir(&folder); // Only this newly created, empty folder.
            return Err(e);
        }
        self.config = next;
        Ok(id)
    }

    pub fn session(&self, id: &str) -> Result<&Session> {
        self.config
            .sessions
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| "Session not found.".into())
    }

    pub fn start(&mut self, id: &str) -> Result<()> {
        let session = self.session(id)?;
        if fs::canonicalize(&session.folder).ok().as_ref() != Some(&session.folder) {
            return Err("This session folder is missing or has moved.".into());
        }
        tempfile::NamedTempFile::new_in(&session.folder)
            .map_err(|e| format!("Session folder is not writable: {e}"))?;
        let source = self
            .config
            .source_dir
            .as_ref()
            .ok_or("Set up a screenshot source first.")?;
        let baseline = scan(source)?;
        if self.monitoring {
            // Files already arriving keep the session that was active when detected.
            self.discover_at(Instant::now())?;
        }
        let mut next = self.config.clone();
        next.active_session_id = Some(id.into());
        self.save(&next)?;
        if !self.monitoring {
            self.seen = baseline.into_iter().map(|(p, _)| p).collect();
            self.pending.clear();
        }
        self.config = next;
        self.monitoring = true;
        self.last_error = None;
        Ok(())
    }

    pub fn pause(&mut self) {
        self.monitoring = false;
        self.pending.clear();
    }

    pub fn deletion_preview(&self, id: &str) -> Result<DeletionPreview> {
        let session = self.session(id)?;
        let folder_missing = path_is_missing(&session.folder)?;
        let (file_count, bytes) = if folder_missing {
            (0, 0)
        } else {
            validate_session_folder(session)?;
            recursive_size(&session.folder)?
        };
        Ok(DeletionPreview {
            id: session.id.clone(),
            name: session.name.clone(),
            folder: session.folder.clone(),
            file_count,
            bytes,
            is_active: self.monitoring && self.config.active_session_id.as_deref() == Some(id),
            folder_missing,
        })
    }

    pub fn delete_session(&mut self, id: &str) -> Result<()> {
        let folder = self.session(id)?.folder.clone();
        if path_is_missing(&folder)? {
            return self.remove_session_records(&HashSet::from([id.to_owned()]));
        }
        self.delete_session_with(id, |folder| {
            trash::delete(folder).map_err(|e| e.to_string())
        })
    }

    fn remove_session_records(&mut self, ids: &HashSet<String>) -> Result<()> {
        let removed_active = self
            .config
            .active_session_id
            .as_ref()
            .is_some_and(|id| ids.contains(id));
        if removed_active {
            self.pause();
        } else {
            self.pending
                .retain(|_, pending| !ids.contains(&pending.session_id));
        }
        let mut next = self.config.clone();
        next.sessions.retain(|session| !ids.contains(&session.id));
        if removed_active {
            next.active_session_id = None;
        }
        self.save(&next)?;
        self.config = next;
        Ok(())
    }

    pub fn reconcile_missing_managed_sessions(&mut self) -> Result<()> {
        let managed_root = self.default_storage_dir();
        let mut missing = HashSet::new();
        for session in &self.config.sessions {
            if session
                .folder
                .parent()
                .is_some_and(|parent| same_path(parent, &managed_root))
                && has_valid_session_folder_name(session)
                && path_is_missing(&session.folder)?
            {
                missing.insert(session.id.clone());
            }
        }
        if !missing.is_empty() {
            self.remove_session_records(&missing)?;
        }
        if self.config.managed_storage
            && self
                .config
                .storage_dir
                .as_deref()
                .is_some_and(|folder| same_path(folder, &managed_root))
            && path_is_missing(&managed_root)?
        {
            fs::create_dir_all(&managed_root)
                .map_err(|error| format!("Cannot restore ShotSort's session storage: {error}"))?;
        }
        Ok(())
    }

    fn delete_session_with<F>(&mut self, id: &str, move_folder: F) -> Result<()>
    where
        F: FnOnce(&Path) -> std::result::Result<(), String>,
    {
        let session = self.session(id)?.clone();
        validate_session_folder(&session)?;
        if self.monitoring && self.config.active_session_id.as_deref() == Some(id) {
            self.pause();
        } else {
            self.pending.retain(|_, pending| pending.session_id != id);
        }

        let original = self.config.clone();
        let mut journaled = original.clone();
        journaled.pending_delete = Some(session.clone());
        self.save(&journaled)?;
        self.config = journaled;

        if let Err(error) = move_folder(&session.folder) {
            let rollback = self.save(&original);
            self.config = original;
            return match rollback {
                Ok(()) => Err(format!("Could not move the session to the Recycle Bin: {error}. Nothing was deleted.")),
                Err(save_error) => Err(format!("Could not move the session to the Recycle Bin: {error}. The folder is safe, but ShotSort could not clear its recovery record: {save_error}")),
            };
        }

        let mut final_config = self.config.clone();
        final_config.sessions.retain(|session| session.id != id);
        if final_config.active_session_id.as_deref() == Some(id) {
            final_config.active_session_id = None;
        }
        final_config.pending_delete = None;
        self.config = final_config.clone();
        self.save(&final_config).map_err(|error| {
            format!("The folder was moved to the Recycle Bin, but session cleanup could not be saved: {error}. Restart ShotSort to finish cleanup.")
        })
    }

    pub fn discover(&mut self) -> Result<()> {
        self.discover_at(Instant::now())
    }

    fn discover_at(&mut self, now: Instant) -> Result<()> {
        if !self.monitoring {
            return Ok(());
        }
        let source = self
            .config
            .source_dir
            .as_ref()
            .ok_or("Missing screenshot source")?;
        if fs::canonicalize(source).ok().as_ref() != Some(source) {
            return Err(
                "The screenshot source moved or became unavailable. Routing is paused.".into(),
            );
        }
        let files = scan(source)?;
        let current: HashSet<_> = files.iter().map(|(path, _)| path.clone()).collect();
        self.seen.retain(|path| current.contains(path));
        self.pending.retain(|path, _| current.contains(path));
        let id = self
            .config
            .active_session_id
            .clone()
            .ok_or("No active session")?;
        for (path, stamp) in files {
            if self.seen.insert(path.clone()) {
                self.pending.insert(
                    path,
                    Pending {
                        session_id: id.clone(),
                        stamp,
                        stable_since: now,
                        attempts: 0,
                    },
                );
            }
        }
        Ok(())
    }

    pub fn tick(&mut self) {
        self.tick_at(Instant::now());
    }

    fn tick_at(&mut self, now: Instant) {
        if !self.monitoring {
            return;
        }
        let paths: Vec<_> = self.pending.keys().cloned().collect();
        for path in paths {
            let Some(pending) = self.pending.get_mut(&path) else {
                continue;
            };
            let current = match stamp(&path) {
                Ok(value) => value,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    self.pending.remove(&path);
                    continue;
                }
                Err(e) => {
                    self.last_error = Some(format!(
                        "Cannot read {}: {e}. The original is untouched.",
                        path.display()
                    ));
                    self.pending.remove(&path);
                    continue;
                }
            };
            if current != pending.stamp || current.bytes == 0 {
                pending.stamp = current;
                pending.stable_since = now;
                continue;
            }
            if now.duration_since(pending.stable_since) < SETTLE_TIME {
                continue;
            }
            let session_id = pending.session_id.clone();
            let expected = pending.stamp.clone();
            let result = self
                .session(&session_id)
                .and_then(|session| move_screenshot(&path, &session.folder, &expected));
            match result {
                Ok(warning) => {
                    self.pending.remove(&path);
                    self.seen.remove(&path);
                    if let Some(warning) = warning {
                        self.last_error = Some(warning);
                        self.seen.insert(path);
                    }
                }
                Err(error) => {
                    self.last_error = Some(error);
                    if let Some(pending) = self.pending.get_mut(&path) {
                        pending.attempts += 1;
                        pending.stable_since = now;
                        if pending.attempts >= 3 {
                            self.pending.remove(&path);
                        }
                    }
                }
            }
        }
    }

    pub fn screenshot_path(&self, session_id: &str, name: &str) -> Result<PathBuf> {
        let session = self.session(session_id)?;
        if Path::new(name).file_name().and_then(|s| s.to_str()) != Some(name)
            || !is_image(Path::new(name))
        {
            return Err("Invalid screenshot name.".into());
        }
        let path = session.folder.join(name);
        stamp(&path).map_err(|e| format!("Screenshot is unavailable: {e}"))?;
        let resolved = fs::canonicalize(&path).map_err(|e| e.to_string())?;
        if resolved.parent() != Some(session.folder.as_path()) {
            return Err("Screenshot is outside the session folder.".into());
        }
        Ok(resolved)
    }

    #[cfg(test)]
    pub fn snapshot(&self, selected_id: Option<&str>) -> Snapshot {
        self.snapshot_with_storage(selected_id, false)
    }

    pub fn snapshot_with_storage(
        &self,
        selected_id: Option<&str>,
        include_storage: bool,
    ) -> Snapshot {
        let mut screenshots = Vec::new();
        let sessions = self
            .config
            .sessions
            .iter()
            .map(|session| {
                let (files, error) = match scan(&session.folder) {
                    Ok(files) => (files, None),
                    Err(e) => (Vec::new(), Some(e)),
                };
                let count = files.len();
                let bytes = files.iter().map(|(_, stamp)| stamp.bytes).sum();
                let (storage_file_count, storage_bytes) = if include_storage && error.is_none() {
                    recursive_size(&session.folder).unwrap_or((count, bytes))
                } else {
                    (count, bytes)
                };
                if selected_id == Some(session.id.as_str()) {
                    screenshots = files
                        .into_iter()
                        .map(|(path, stamp)| Screenshot {
                            name: path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into(),
                            bytes: stamp.bytes,
                            modified_at: millis(stamp.modified),
                        })
                        .collect();
                    screenshots.sort_by(|a, b| {
                        b.modified_at
                            .cmp(&a.modified_at)
                            .then_with(|| a.name.cmp(&b.name))
                    });
                }
                SessionView {
                    session: session.clone(),
                    count,
                    bytes,
                    storage_file_count,
                    storage_bytes,
                    error,
                }
            })
            .collect();
        Snapshot {
            source_dir: self.config.source_dir.clone(),
            storage_dir: self.config.storage_dir.clone(),
            managed_storage: self.config.managed_storage,
            default_storage_dir: self.default_storage_dir(),
            active_session_id: self.config.active_session_id.clone(),
            monitoring: self.monitoring,
            pending_count: self.pending.len(),
            sessions,
            screenshots,
            last_error: self.last_error.clone(),
        }
    }
}

fn move_screenshot(source: &Path, folder: &Path, expected: &Stamp) -> Result<Option<String>> {
    if fs::canonicalize(folder).ok().as_deref() != Some(folder) {
        return Err(
            "Session folder moved or is unavailable. The screenshot remains in its source folder."
                .into(),
        );
    }
    let operation = || -> io::Result<Option<String>> {
        if stamp(source)? != *expected {
            return Err(io::Error::other("Screenshot is still changing"));
        }
        let mut input = File::open(source)?;
        let mut temporary = tempfile::NamedTempFile::new_in(folder)?;
        let copied = io::copy(&mut input, temporary.as_file_mut())?;
        temporary.as_file().sync_all()?;
        if copied != expected.bytes || stamp(source)? != *expected {
            return Err(io::Error::other(
                "Screenshot changed while being saved; original retained",
            ));
        }
        drop(input);
        let name = source
            .file_name()
            .ok_or_else(|| io::Error::other("Missing filename"))?;
        let stem = source.file_stem().unwrap_or_default().to_string_lossy();
        let extension = source.extension().unwrap_or_default().to_string_lossy();
        let mut index = 0;
        loop {
            let target = if index == 0 {
                folder.join(name)
            } else {
                folder.join(format!("{stem} ({index}).{extension}"))
            };
            // Atomic, no-overwrite publication, including when another writer wins a filename race.
            match temporary.persist_noclobber(&target) {
                Ok(saved) => {
                    drop(saved);
                    break;
                }
                Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                    temporary = error.file;
                    index += 1;
                    if index > 10_000 {
                        return Err(io::Error::other("Too many matching filenames"));
                    }
                }
                Err(error) => return Err(error.error),
            }
        }
        // Publication succeeded: never retry a copy if source verification now fails.
        match stamp(source) {
            Ok(current) if current == *expected => {},
            _ => return Ok(Some("Screenshot saved, but its source changed or became unavailable. No source was deleted; review the source folder.".into())),
        }
        match fs::remove_file(source) {
            Ok(()) => Ok(None),
            Err(e) => Ok(Some(format!("Screenshot saved, but its source could not be removed: {e}. Both copies were kept."))),
        }
    };
    operation().map_err(|e| {
        format!(
            "Could not store {}: {e}. The source has not been deleted.",
            source.file_name().unwrap_or_default().to_string_lossy()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        source: PathBuf,
        destination: PathBuf,
        storage: Storage,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("screenshots");
            let destination = dir.path().join("assignments");
            fs::create_dir(&source).unwrap();
            fs::create_dir(&destination).unwrap();
            let mut storage = Storage::load(dir.path().join("config/settings.json")).unwrap();
            storage
                .configure(source.to_str().unwrap(), destination.to_str().unwrap())
                .unwrap();
            Self {
                _dir: dir,
                source,
                destination,
                storage,
            }
        }
        fn settle(&mut self) {
            let now = Instant::now();
            self.storage.discover_at(now).unwrap();
            self.storage
                .tick_at(now + SETTLE_TIME + Duration::from_millis(1));
        }
    }

    #[test]
    fn routes_only_new_images_and_preserves_contents() {
        let mut f = Fixture::new();
        fs::write(f.source.join("old.png"), b"old screenshot").unwrap();
        let id = f.storage.create_session("DBMS Lab").unwrap();
        f.storage.start(&id).unwrap();
        fs::write(f.source.join("new.PNG"), b"new screenshot bytes").unwrap();
        fs::write(f.source.join("notes.txt"), b"leave me alone").unwrap();
        f.settle();
        let folder = &f.storage.session(&id).unwrap().folder;
        assert_eq!(
            fs::read(folder.join("new.PNG")).unwrap(),
            b"new screenshot bytes"
        );
        assert!(!f.source.join("new.PNG").exists());
        assert!(f.source.join("old.png").exists());
        assert!(f.source.join("notes.txt").exists());
        let snapshot = f.storage.snapshot(Some(&id));
        assert_eq!(snapshot.sessions[0].count, 1);
        assert_eq!(snapshot.screenshots[0].bytes, 20);
    }

    #[test]
    fn paused_files_stay_put_and_restart_restores_sessions_paused() {
        let mut f = Fixture::new();
        let id = f.storage.create_session("Assignment").unwrap();
        f.storage.start(&id).unwrap();
        f.storage.pause();
        fs::write(f.source.join("paused.png"), b"keep").unwrap();
        f.storage.start(&id).unwrap();
        f.settle();
        assert!(f.source.join("paused.png").exists());
        let restored = Storage::load(f.storage.config_path.clone()).unwrap();
        assert_eq!(restored.config.active_session_id, Some(id));
        assert_eq!(restored.config.sessions.len(), 1);
        assert!(!restored.monitoring);
    }

    #[test]
    fn switching_sessions_keeps_in_flight_files_with_original_session() {
        let mut f = Fixture::new();
        let first = f.storage.create_session("First").unwrap();
        let second = f.storage.create_session("Second").unwrap();
        f.storage.start(&first).unwrap();
        fs::write(f.source.join("first.png"), b"first").unwrap();
        f.storage.start(&second).unwrap();
        fs::write(f.source.join("second.png"), b"second").unwrap();
        f.settle();
        assert!(f
            .storage
            .session(&first)
            .unwrap()
            .folder
            .join("first.png")
            .exists());
        assert!(f
            .storage
            .session(&second)
            .unwrap()
            .folder
            .join("second.png")
            .exists());
    }

    #[test]
    fn filename_collisions_never_overwrite() {
        let mut f = Fixture::new();
        let id = f.storage.create_session("Lab").unwrap();
        let folder = f.storage.session(&id).unwrap().folder.clone();
        fs::write(folder.join("shot.png"), b"original").unwrap();
        f.storage.start(&id).unwrap();
        fs::write(f.source.join("shot.png"), b"second").unwrap();
        f.settle();
        assert_eq!(fs::read(folder.join("shot.png")).unwrap(), b"original");
        assert_eq!(fs::read(folder.join("shot (1).png")).unwrap(), b"second");
        fs::write(f.source.join("shot.png"), b"third").unwrap();
        f.settle();
        assert_eq!(fs::read(folder.join("shot (2).png")).unwrap(), b"third");
    }

    #[test]
    fn partially_written_images_wait_until_stable() {
        let mut f = Fixture::new();
        let id = f.storage.create_session("Lab").unwrap();
        f.storage.start(&id).unwrap();
        let path = f.source.join("shot.png");
        fs::write(&path, b"part").unwrap();
        let now = Instant::now();
        f.storage.discover_at(now).unwrap();
        fs::write(&path, b"complete image").unwrap();
        f.storage.tick_at(now + SETTLE_TIME);
        assert!(path.exists());
        f.storage.tick_at(now + SETTLE_TIME * 2);
        assert!(!path.exists());
        assert_eq!(
            fs::read(f.storage.session(&id).unwrap().folder.join("shot.png")).unwrap(),
            b"complete image"
        );
    }

    #[test]
    fn rejects_nested_folders_and_changes_while_running() {
        let mut f = Fixture::new();
        let nested = f.source.join("nested");
        fs::create_dir(&nested).unwrap();
        assert!(f
            .storage
            .configure(f.source.to_str().unwrap(), nested.to_str().unwrap())
            .is_err());
        let id = f.storage.create_session("Lab").unwrap();
        f.storage.start(&id).unwrap();
        assert!(f
            .storage
            .configure(f.source.to_str().unwrap(), f.destination.to_str().unwrap())
            .is_err());
    }

    #[test]
    fn missing_destination_retains_original_and_reports_error() {
        let mut f = Fixture::new();
        let id = f.storage.create_session("Lab").unwrap();
        f.storage.start(&id).unwrap();
        fs::remove_dir(&f.storage.session(&id).unwrap().folder).unwrap();
        fs::write(f.source.join("shot.png"), b"keep safe").unwrap();
        f.settle();
        assert!(f.source.join("shot.png").exists());
        assert!(f.storage.last_error.is_some());
    }

    #[test]
    fn rejects_invalid_names_and_path_traversal() {
        let mut f = Fixture::new();
        assert!(f.storage.create_session("   ").is_err());
        assert!(f.storage.create_session(&"x".repeat(81)).is_err());
        let id = f.storage.create_session("Lab").unwrap();
        assert!(f.storage.screenshot_path(&id, "../outside.png").is_err());
        assert!(f.storage.screenshot_path(&id, "program.exe").is_err());
    }

    #[test]
    fn damaged_settings_are_not_silently_reset() {
        let f = Fixture::new();
        fs::write(&f.storage.config_path, "broken json").unwrap();
        assert!(Storage::load(f.storage.config_path.clone()).is_err());
        assert_eq!(
            fs::read_to_string(&f.storage.config_path).unwrap(),
            "broken json"
        );
    }

    #[test]
    fn managed_storage_and_quick_sessions_need_no_precreated_destination() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("screenshots");
        fs::create_dir(&source).unwrap();
        let config_path = temp.path().join("app-data/sessions.json");
        let mut storage = Storage::load(config_path.clone()).unwrap();
        assert!(!storage.default_storage_dir().exists());
        storage.configure(source.to_str().unwrap(), "").unwrap();
        assert!(storage.config.managed_storage);
        assert!(storage.config.storage_dir.as_ref().unwrap().is_dir());
        let first = storage.create_quick_session().unwrap();
        let second = storage.create_quick_session().unwrap();
        assert_ne!(first, second);
        assert_eq!(storage.session(&first).unwrap().name, "Quick session 1");
        assert_eq!(storage.session(&second).unwrap().name, "Quick session 2");
        assert!(!storage.monitoring);
        let folder = storage.session(&first).unwrap().folder.clone();
        fs::write(folder.join("keep.png"), b"keep after closing").unwrap();
        drop(storage);
        let restored = Storage::load(config_path).unwrap();
        assert!(restored.config.managed_storage);
        assert_eq!(restored.config.sessions.len(), 2);
        assert_eq!(
            fs::read(folder.join("keep.png")).unwrap(),
            b"keep after closing"
        );
        assert!(!restored.monitoring);
    }

    #[test]
    fn switching_to_managed_storage_does_not_relocate_old_sessions() {
        let mut f = Fixture::new();
        let id = f.storage.create_session("Existing assignment").unwrap();
        let original = f.storage.session(&id).unwrap().folder.clone();
        fs::write(original.join("keep.png"), b"original bytes").unwrap();
        f.storage.configure(f.source.to_str().unwrap(), "").unwrap();
        let quick = f.storage.create_quick_session().unwrap();
        assert_eq!(f.storage.session(&id).unwrap().folder, original);
        assert_eq!(
            fs::read(original.join("keep.png")).unwrap(),
            b"original bytes"
        );
        assert!(f
            .storage
            .session(&quick)
            .unwrap()
            .folder
            .starts_with(f.storage.config.storage_dir.as_ref().unwrap()));
    }

    #[test]
    fn old_configuration_keeps_custom_storage() {
        let f = Fixture::new();
        let mut old = serde_json::to_value(&f.storage.config).unwrap();
        old.as_object_mut().unwrap().remove("managedStorage");
        fs::write(&f.storage.config_path, serde_json::to_vec(&old).unwrap()).unwrap();
        let restored = Storage::load(f.storage.config_path.clone()).unwrap();
        assert!(!restored.config.managed_storage);
        assert_eq!(restored.config.storage_dir, f.storage.config.storage_dir);
    }

    #[test]
    fn failed_managed_folder_creation_preserves_previous_settings() {
        let mut f = Fixture::new();
        let original = f.storage.config.storage_dir.clone();
        fs::write(f.storage.default_storage_dir(), b"not a directory").unwrap();
        assert!(f.storage.configure(f.source.to_str().unwrap(), "").is_err());
        assert_eq!(f.storage.config.storage_dir, original);
        assert!(!f.storage.config.managed_storage);
    }

    #[test]
    fn deletion_preview_counts_every_file_recursively() {
        let mut f = Fixture::new();
        let id = f.storage.create_session("Finished assignment").unwrap();
        let folder = f.storage.session(&id).unwrap().folder.clone();
        fs::write(folder.join("shot.png"), b"1234").unwrap();
        fs::create_dir(folder.join("notes")).unwrap();
        fs::write(folder.join("notes/readme.txt"), b"123456").unwrap();
        let preview = f.storage.deletion_preview(&id).unwrap();
        assert_eq!(preview.file_count, 2);
        assert_eq!(preview.bytes, 10);
        assert!(!preview.is_active);
        let snapshot = f.storage.snapshot_with_storage(Some(&id), true);
        assert_eq!(snapshot.sessions[0].count, 1);
        assert_eq!(snapshot.sessions[0].bytes, 4);
        assert_eq!(snapshot.sessions[0].storage_file_count, 2);
        assert_eq!(snapshot.sessions[0].storage_bytes, 10);
    }

    #[test]
    fn deleting_an_active_session_pauses_and_removes_its_record() {
        let mut f = Fixture::new();
        let id = f.storage.create_session("Finished assignment").unwrap();
        let folder = f.storage.session(&id).unwrap().folder.clone();
        fs::write(folder.join("shot.png"), b"finished work").unwrap();
        f.storage.start(&id).unwrap();
        assert!(f.storage.deletion_preview(&id).unwrap().is_active);
        f.storage
            .delete_session_with(&id, |folder| {
                fs::remove_dir_all(folder).map_err(|e| e.to_string())
            })
            .unwrap();
        assert!(!f.storage.monitoring);
        assert!(f.storage.config.active_session_id.is_none());
        assert!(f.storage.session(&id).is_err());
        assert!(!folder.exists());
        let restored = Storage::load(f.storage.config_path.clone()).unwrap();
        assert!(restored.session(&id).is_err());
    }

    #[test]
    fn failed_recycle_operation_keeps_folder_and_session_but_stays_paused() {
        let mut f = Fixture::new();
        let id = f.storage.create_session("Keep me").unwrap();
        let folder = f.storage.session(&id).unwrap().folder.clone();
        fs::write(folder.join("shot.png"), b"safe").unwrap();
        f.storage.start(&id).unwrap();
        let result = f
            .storage
            .delete_session_with(&id, |_| Err("Recycle Bin unavailable".into()));
        assert!(result.is_err());
        assert!(!f.storage.monitoring);
        assert!(f.storage.session(&id).is_ok());
        assert_eq!(fs::read(folder.join("shot.png")).unwrap(), b"safe");
        assert!(f.storage.config.pending_delete.is_none());
        let restored = Storage::load(f.storage.config_path.clone()).unwrap();
        assert!(restored.session(&id).is_ok());
    }

    #[test]
    fn deleting_an_inactive_session_does_not_interrupt_current_routing() {
        let mut f = Fixture::new();
        let old = f.storage.create_session("Old").unwrap();
        let current = f.storage.create_session("Current").unwrap();
        f.storage.start(&old).unwrap();
        let source = f.source.join("in-flight.png");
        fs::write(&source, b"stay in source").unwrap();
        f.storage.discover().unwrap();
        f.storage.start(&current).unwrap();
        f.storage
            .delete_session_with(&old, |folder| {
                fs::remove_dir_all(folder).map_err(|e| e.to_string())
            })
            .unwrap();
        assert!(f.storage.monitoring);
        assert_eq!(
            f.storage.config.active_session_id.as_deref(),
            Some(current.as_str())
        );
        assert!(source.exists());
        assert!(f
            .storage
            .pending
            .values()
            .all(|pending| pending.session_id != old));
    }

    #[test]
    fn interrupted_successful_deletion_is_finished_on_restart() {
        let mut f = Fixture::new();
        let id = f.storage.create_session("Finished").unwrap();
        let session = f.storage.session(&id).unwrap().clone();
        let mut journaled = f.storage.config.clone();
        journaled.pending_delete = Some(session.clone());
        write_config(&f.storage.config_path, &journaled).unwrap();
        fs::remove_dir_all(&session.folder).unwrap();
        let restored = Storage::load(f.storage.config_path.clone()).unwrap();
        assert!(restored.session(&id).is_err());
        assert!(restored.config.pending_delete.is_none());
    }

    #[test]
    fn interrupted_deletion_before_recycling_preserves_the_session() {
        let mut f = Fixture::new();
        let id = f.storage.create_session("Keep after interruption").unwrap();
        let session = f.storage.session(&id).unwrap().clone();
        fs::write(session.folder.join("keep.png"), b"still here").unwrap();
        let mut journaled = f.storage.config.clone();
        journaled.pending_delete = Some(session.clone());
        write_config(&f.storage.config_path, &journaled).unwrap();
        let restored = Storage::load(f.storage.config_path.clone()).unwrap();
        assert!(restored.session(&id).is_ok());
        assert_eq!(
            fs::read(session.folder.join("keep.png")).unwrap(),
            b"still here"
        );
        assert!(restored.config.pending_delete.is_none());
    }

    #[test]
    fn deletion_rejects_a_session_record_pointing_at_an_arbitrary_folder() {
        let mut f = Fixture::new();
        let id = f.storage.create_session("Modified record").unwrap();
        let arbitrary = f.destination.join("session-personal-files");
        fs::create_dir(&arbitrary).unwrap();
        fs::write(arbitrary.join("keep.txt"), b"never delete").unwrap();
        f.storage
            .config
            .sessions
            .iter_mut()
            .find(|session| session.id == id)
            .unwrap()
            .folder = arbitrary.clone();
        let mut called = false;
        let result = f.storage.delete_session_with(&id, |_| {
            called = true;
            Ok(())
        });
        assert!(result.is_err());
        assert!(!called);
        assert_eq!(
            fs::read(arbitrary.join("keep.txt")).unwrap(),
            b"never delete"
        );
    }

    #[test]
    fn missing_managed_session_is_removed_and_storage_root_is_restored() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("screenshots");
        fs::create_dir(&source).unwrap();
        let config_path = temp.path().join("app-data/sessions.json");
        let mut storage = Storage::load(config_path.clone()).unwrap();
        storage.configure(source.to_str().unwrap(), "").unwrap();
        let id = storage.create_session("Deleted in File Explorer").unwrap();
        let session_folder = storage.session(&id).unwrap().folder.clone();
        let managed_root = storage.default_storage_dir();
        storage.start(&id).unwrap();
        fs::remove_dir_all(&managed_root).unwrap();

        storage.reconcile_missing_managed_sessions().unwrap();

        assert!(storage.session(&id).is_err());
        assert!(storage.config.active_session_id.is_none());
        assert!(!storage.monitoring);
        assert!(managed_root.is_dir());
        assert!(!session_folder.exists());
        let restored = Storage::load(config_path).unwrap();
        assert!(restored.session(&id).is_err());
    }

    #[test]
    fn missing_custom_session_is_preserved_until_record_is_removed() {
        let mut f = Fixture::new();
        let id = f.storage.create_session("Disconnected drive").unwrap();
        let folder = f.storage.session(&id).unwrap().folder.clone();
        fs::remove_dir(&folder).unwrap();

        f.storage.reconcile_missing_managed_sessions().unwrap();
        assert!(f.storage.session(&id).is_ok());
        let preview = f.storage.deletion_preview(&id).unwrap();
        assert!(preview.folder_missing);
        assert_eq!(preview.file_count, 0);
        f.storage.delete_session(&id).unwrap();

        assert!(f.storage.session(&id).is_err());
        let restored = Storage::load(f.storage.config_path.clone()).unwrap();
        assert!(restored.session(&id).is_err());
    }
}
