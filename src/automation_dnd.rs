// SPDX-License-Identifier: GPL-3.0-or-later

//! Compositor-owned drag-and-drop payloads used by the automation API.
//!
//! This module deliberately contains no event-loop or pointer-grab policy.  It
//! validates and prepares payloads, implements Smithay's server-side [`Source`]
//! abstraction, and keeps the small completion history exposed to Scheme.

use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use smithay::input::dnd::{DndAction, Source, SourceMetadata};
use smithay::utils::IsAlive;

pub const AUTOMATION_RESULT_LIMIT: usize = 128;
pub const URI_LIST_MIME: &str = "text/uri-list";
pub const TEXT_MIME_UTF8: &str = "text/plain;charset=utf-8";
pub const TEXT_MIME: &str = "text/plain";

pub type AutomationToken = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationOperation {
    DropFiles,
    DropText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationStatus {
    Pending,
    Accepted,
    Rejected,
    NoTarget,
    Cancelled,
    UnsupportedTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutomationResult {
    pub token: AutomationToken,
    pub operation: AutomationOperation,
    pub status: AutomationStatus,
}

/// Thread-safe, bounded completion history. Updating an existing token moves it
/// to the newest position, which makes late terminal events authoritative.
#[derive(Debug, Clone)]
pub struct AutomationResults {
    inner: Arc<Mutex<VecDeque<AutomationResult>>>,
    next_token: Arc<AtomicU64>,
    limit: usize,
}

impl Default for AutomationResults {
    fn default() -> Self {
        Self::new(AUTOMATION_RESULT_LIMIT)
    }
}

impl AutomationResults {
    pub fn new(limit: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(limit))),
            next_token: Arc::new(AtomicU64::new(1)),
            limit,
        }
    }

    pub fn next_token(&self) -> AutomationToken {
        // Token zero is reserved for "no token" at API boundaries.
        loop {
            let token = self.next_token.fetch_add(1, Ordering::Relaxed);
            if token != 0 {
                return token;
            }
        }
    }

    /// Allocate a request token and immediately make it observable as queued.
    pub fn allocate(&self, operation: AutomationOperation) -> AutomationToken {
        let token = self.next_token();
        self.record(AutomationResult {
            token,
            operation,
            status: AutomationStatus::Pending,
        });
        token
    }

    pub fn record(&self, result: AutomationResult) {
        if self.limit == 0 {
            return;
        }
        let mut results = lock(&self.inner);
        if let Some(index) = results.iter().position(|item| item.token == result.token) {
            results.remove(index);
        }
        results.push_back(result);
        while results.len() > self.limit {
            results.pop_front();
        }
    }

    pub fn get(&self, token: AutomationToken) -> Option<AutomationResult> {
        lock(&self.inner)
            .iter()
            .find(|item| item.token == token)
            .copied()
    }

    #[cfg(test)]
    pub fn recent(&self) -> Vec<AutomationResult> {
        lock(&self.inner).iter().copied().collect()
    }
}

/// The single result registry shared by compositor state and IPC gsubrs.
pub fn automation_results() -> &'static AutomationResults {
    static RESULTS: OnceLock<AutomationResults> = OnceLock::new();
    RESULTS.get_or_init(AutomationResults::default)
}

#[derive(Debug, thiserror::Error)]
pub enum DropFilesError {
    #[error("at least one file is required")]
    Empty,
    #[error("drop path is not absolute: {0}")]
    NotAbsolute(PathBuf),
    #[error("drop path does not name a regular file: {0}")]
    NotAFile(PathBuf),
    #[error("cannot resolve drop path {path}: {source}")]
    Canonicalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Validate every file and return a canonical `text/uri-list` payload.
///
/// URI-list records are UTF-8 percent-encoded and CRLF terminated as required
/// by RFC 2483. Validation is all-or-nothing, including multi-file drops.
pub fn build_uri_list<I, P>(paths: I) -> Result<Vec<u8>, DropFilesError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .map(|path| path.as_ref().to_path_buf())
        .collect();
    if paths.is_empty() {
        return Err(DropFilesError::Empty);
    }

    let mut payload = String::new();
    for path in paths {
        if !path.is_absolute() {
            return Err(DropFilesError::NotAbsolute(path));
        }
        let canonical = fs::canonicalize(&path).map_err(|source| DropFilesError::Canonicalize {
            path: path.clone(),
            source,
        })?;
        if !canonical
            .metadata()
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            return Err(DropFilesError::NotAFile(path));
        }

        payload.push_str("file://");
        payload.push_str(&percent_encode_path(&canonical));
        payload.push_str("\r\n");
    }
    Ok(payload.into_bytes())
}

fn percent_encode_path(path: &Path) -> String {
    let display = path.to_string_lossy();
    let mut encoded = String::with_capacity(display.len());
    for byte in display.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            encoded.push(char::from(*byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[derive(Debug)]
struct SourceState {
    selected_action: DndAction,
    dropped: bool,
    terminal: bool,
}

/// A Smithay DnD source whose bytes are owned by the compositor.
#[derive(Debug, Clone)]
pub struct AutomationDndSource {
    payload: Arc<[u8]>,
    mime_types: Arc<[String]>,
    operation: AutomationOperation,
    token: AutomationToken,
    results: AutomationResults,
    alive: Arc<AtomicBool>,
    state: Arc<Mutex<SourceState>>,
}

impl AutomationDndSource {
    pub fn files<I, P>(
        paths: I,
        token: AutomationToken,
        results: AutomationResults,
    ) -> Result<Self, DropFilesError>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        Ok(Self::new(
            build_uri_list(paths)?,
            vec![URI_LIST_MIME.to_owned()],
            AutomationOperation::DropFiles,
            token,
            results,
        ))
    }

    pub fn text(text: String, token: AutomationToken, results: AutomationResults) -> Self {
        Self::new(
            text.into_bytes(),
            vec![TEXT_MIME_UTF8.to_owned(), TEXT_MIME.to_owned()],
            AutomationOperation::DropText,
            token,
            results,
        )
    }

    fn new(
        payload: Vec<u8>,
        mime_types: Vec<String>,
        operation: AutomationOperation,
        token: AutomationToken,
        results: AutomationResults,
    ) -> Self {
        Self {
            payload: payload.into(),
            mime_types: mime_types.into(),
            operation,
            token,
            results,
            alive: Arc::new(AtomicBool::new(true)),
            state: Arc::new(Mutex::new(SourceState {
                selected_action: DndAction::None,
                dropped: false,
                terminal: false,
            })),
        }
    }

    pub fn token(&self) -> AutomationToken {
        self.token
    }

    pub fn operation(&self) -> AutomationOperation {
        self.operation
    }

    fn complete(&self, status: AutomationStatus) {
        let mut state = lock(&self.state);
        if state.terminal {
            return;
        }
        state.terminal = true;
        self.alive.store(false, Ordering::Release);
        self.results.record(AutomationResult {
            token: self.token,
            operation: self.operation,
            status,
        });
    }
}

impl IsAlive for AutomationDndSource {
    fn alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
}

impl Source for AutomationDndSource {
    fn metadata(&self) -> Option<SourceMetadata> {
        let mut metadata = SourceMetadata {
            mime_types: self.mime_types.to_vec(),
            ..SourceMetadata::default()
        };
        metadata.dnd_actions.push(DndAction::Copy);
        Some(metadata)
    }

    fn choose_action(&self, action: DndAction) {
        lock(&self.state).selected_action = action;
    }

    fn send(&self, mime_type: &str, fd: OwnedFd) {
        if !self.mime_types.iter().any(|offered| offered == mime_type) {
            return;
        }
        let payload = Arc::clone(&self.payload);
        std::thread::spawn(move || {
            let mut file = fs::File::from(fd);
            if let Err(error) = file.write_all(&payload) {
                tracing::warn!(?error, "failed to send compositor DnD payload");
            }
        });
    }

    fn drop_performed(&self) {
        lock(&self.state).dropped = true;
    }

    fn cancel(&self) {
        self.complete(AutomationStatus::Cancelled);
    }

    fn finished(&self) {
        let state = lock(&self.state);
        let accepted = state.dropped && state.selected_action == DndAction::Copy;
        drop(state);
        self.complete(if accepted {
            AutomationStatus::Accepted
        } else {
            AutomationStatus::Rejected
        });
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("minde-dnd-{}-{unique}", std::process::id()))
    }

    #[test]
    fn uri_list_is_encoded_and_crlf_terminated() {
        let dir = test_dir();
        fs::create_dir_all(&dir).unwrap();
        let first = dir.join("résumé #1.pdf");
        let second = dir.join("plain.txt");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();

        let payload = String::from_utf8(build_uri_list([&first, &second]).unwrap()).unwrap();
        assert!(payload.contains("r%C3%A9sum%C3%A9%20%231.pdf\r\n"));
        assert!(payload.ends_with("plain.txt\r\n"));
        assert_eq!(payload.lines().count(), 2);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn uri_list_rejects_empty_relative_missing_and_directory_paths() {
        assert!(matches!(
            build_uri_list(Vec::<PathBuf>::new()),
            Err(DropFilesError::Empty)
        ));
        assert!(matches!(
            build_uri_list([PathBuf::from("relative")]),
            Err(DropFilesError::NotAbsolute(_))
        ));

        let dir = test_dir();
        fs::create_dir_all(&dir).unwrap();
        assert!(matches!(
            build_uri_list([dir.join("missing")]),
            Err(DropFilesError::Canonicalize { .. })
        ));
        assert!(matches!(
            build_uri_list([&dir]),
            Err(DropFilesError::NotAFile(_))
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn result_store_updates_and_evicts_oldest() {
        let results = AutomationResults::new(2);
        for (token, status) in [
            (1, AutomationStatus::NoTarget),
            (2, AutomationStatus::Cancelled),
            (1, AutomationStatus::Accepted),
            (3, AutomationStatus::UnsupportedTarget),
        ] {
            results.record(AutomationResult {
                token,
                operation: AutomationOperation::DropFiles,
                status,
            });
        }
        assert!(results.get(2).is_none());
        assert_eq!(results.get(1).unwrap().status, AutomationStatus::Accepted);
        assert_eq!(results.recent().len(), 2);
    }

    #[test]
    fn allocation_is_immediately_observable_as_pending() {
        let results = AutomationResults::default();
        let token = results.allocate(AutomationOperation::DropText);
        assert_ne!(token, 0);
        assert_eq!(
            results.get(token).unwrap().status,
            AutomationStatus::Pending
        );
        assert!(results.get(token + 1).is_none());
    }

    #[test]
    fn source_has_copy_metadata_and_terminal_status() {
        let results = AutomationResults::default();
        let token = results.next_token();
        let source = AutomationDndSource::text("hello".into(), token, results.clone());
        let metadata = source.metadata().unwrap();
        assert_eq!(
            metadata.mime_types,
            vec![TEXT_MIME_UTF8.to_owned(), TEXT_MIME.to_owned()]
        );
        assert_eq!(metadata.dnd_actions.as_slice(), [DndAction::Copy]);
        source.choose_action(DndAction::Copy);
        source.drop_performed();
        source.finished();
        assert!(!source.alive());
        assert_eq!(
            results.get(token).unwrap().status,
            AutomationStatus::Accepted
        );
        source.cancel();
        assert_eq!(
            results.get(token).unwrap().status,
            AutomationStatus::Accepted
        );
    }
}
