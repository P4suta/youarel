//! The watch of the shared folder, through the operating system's notices: FSEvents on macOS, ReadDirectoryChangesW on Windows, and inotify on Linux.
//!
//! Each notice reaches the window as what the core reads: a read, the name of a file that changed, or a rescan.
//! The watch compares no whole path, since FSEvents names the folder by its canonical path and Windows joins names with backslashes.

use notify::event::{EventKind, RemoveKind};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use youarel_core::folder_rules::Seen;

/// A watch of one shared folder, which ends when it drops.
///
/// Dropping a watch on macOS waits for its thread, which may be telling of a change, so its owner drops it with no lock held that the telling takes.
#[derive(Debug)]
pub struct Watch {
    folder: PathBuf,
    /// Whether the watch lost its folder or failed, so the window starts it again.
    lost: Arc<AtomicBool>,
    /// The operating system's watch, kept alive for as long as this value lives.
    _watcher: RecommendedWatcher,
}

impl Watch {
    /// Starts watching `folder` itself, not its subfolders, and tells `tell` of each change on the watch's own thread.
    ///
    /// # Errors
    ///
    /// Returns an error when the folder does not exist or the host refuses another watch.
    pub fn start(folder: &Path, tell: impl Fn(Seen<'_>) + Send + 'static) -> notify::Result<Self> {
        let lost = Arc::new(AtomicBool::new(false));
        let marked = Arc::clone(&lost);
        let mut watcher = notify::recommended_watcher(move |event| report(event, &marked, &tell))?;
        watcher.watch(folder, RecursiveMode::NonRecursive)?;
        Ok(Self {
            folder: folder.to_path_buf(),
            lost,
            _watcher: watcher,
        })
    }

    /// The folder as the settings named it.
    #[must_use]
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Whether the watch lost its folder or failed, so the window starts it again.
    #[must_use]
    pub fn lost(&self) -> bool {
        self.lost.load(Ordering::Relaxed)
    }
}

/// Tells of one notice: a failure or a removed folder as a rescan that marks the watch lost, a notice of lost events as a rescan, a read as a read, and any other change by the name of each file, or as a rescan when it names none.
///
/// FSEvents joins the kinds of changes in one notice, so every kind but a read counts as a change.
fn report(event: notify::Result<Event>, lost: &AtomicBool, tell: &impl Fn(Seen<'_>)) {
    let event = match event {
        Ok(event) if !matches!(event.kind, EventKind::Remove(RemoveKind::Folder)) => event,
        // A failed watch, or one whose folder or a folder inside it went away, may miss what follows, so the window starts it again.
        Ok(_) | Err(_) => {
            lost.store(true, Ordering::Relaxed);
            tell(Seen::Rescan);
            return;
        }
    };
    if event.need_rescan() {
        tell(Seen::Rescan);
    } else if matches!(event.kind, EventKind::Access(_)) {
        tell(Seen::Read);
    } else if event.paths.is_empty() {
        tell(Seen::Rescan);
    } else {
        for path in &event.paths {
            // Every bundle's name is Unicode, so a name that is not names no bundle.
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                tell(Seen::Touched(name));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Watch, report};
    use notify::Event;
    use notify::event::{AccessKind, CreateKind, EventKind, Flag, ModifyKind, RemoveKind};
    use std::cell::RefCell;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;
    use youarel_core::folder_rules::Seen;

    /// How long the test waits for the operating system to tell of a change, which a busy host may take.
    const PATIENCE: Duration = Duration::from_secs(10);
    /// How many notices the test reads before it gives up on the one it waits for.
    const MOST_NOTICES: usize = 64;
    /// The error number of a host out of open files, which Linux answers once inotify reaches its limit of instances.
    const TOO_MANY_OPEN_FILES: i32 = 24;

    /// What a watch told, owned so that it crosses threads.
    #[derive(Debug, Eq, PartialEq)]
    enum Told {
        Read,
        Touched(String),
        Rescan,
    }

    impl From<Seen<'_>> for Told {
        fn from(seen: Seen<'_>) -> Self {
            match seen {
                Seen::Read => Self::Read,
                Seen::Touched(name) => Self::Touched(name.to_owned()),
                Seen::Rescan => Self::Rescan,
            }
        }
    }

    /// Whether the host refused another watch for its limits, as a container's inotify may, which leaves nothing to test there.
    fn refused_by_host(error: &notify::Error) -> bool {
        match &error.kind {
            notify::ErrorKind::MaxFilesWatch => true,
            notify::ErrorKind::Io(io) => io.raw_os_error() == Some(TOO_MANY_OPEN_FILES),
            notify::ErrorKind::Generic(_)
            | notify::ErrorKind::PathNotFound
            | notify::ErrorKind::WatchNotFound
            | notify::ErrorKind::InvalidConfig(_) => false,
        }
    }

    /// A watch of `folder` and what it tells, or `None` on a host that refuses another watch.
    fn started(folder: &Path) -> Option<(Watch, Receiver<Told>)> {
        let (sender, told) = channel();
        let tell = move |seen: Seen<'_>| {
            // A notice after the test stopped reading has no reader, and the test needs none.
            drop(sender.send(Told::from(seen)));
        };
        match Watch::start(folder, tell) {
            Ok(watch) => Some((watch, told)),
            Err(error) if refused_by_host(&error) => {
                eprintln!("this host refuses another watch ({error}), so the test watches nothing");
                None
            }
            Err(error) => panic!("the watch did not start: {error}"),
        }
    }

    /// Whether the watch tells of a change to `name` before it stays silent for the whole patience or tells of too much else.
    fn tells_of(told: &Receiver<Told>, name: &str) -> bool {
        for _ in 0..MOST_NOTICES {
            match told.recv_timeout(PATIENCE) {
                Ok(Told::Touched(touched)) if touched == name => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        false
    }

    #[test]
    fn a_bundle_written_into_the_folder_is_told_by_its_name() {
        let folder = tempfile::tempdir().unwrap();
        let Some((watch, told)) = started(folder.path()) else {
            return;
        };
        assert_eq!(watch.folder(), folder.path());
        youarel_store::fs::replace(&folder.path().join("ab.youarel"), b"a bundle").unwrap();
        assert!(tells_of(&told, "ab.youarel"));
        assert!(!watch.lost());
    }

    #[test]
    fn a_folder_that_does_not_exist_has_no_watch() {
        let folder = tempfile::tempdir().unwrap();
        let error = Watch::start(&folder.path().join("missing"), |_| {}).unwrap_err();
        assert!(!refused_by_host(&error), "{error}");
    }

    /// What `report` tells of one notice, and whether it marks the watch lost.
    fn reported(event: notify::Result<Event>) -> (Vec<Told>, bool) {
        let lost = AtomicBool::new(false);
        let told = RefCell::new(Vec::new());
        report(event, &lost, &|seen: Seen<'_>| {
            told.borrow_mut().push(Told::from(seen));
        });
        (told.into_inner(), lost.into_inner())
    }

    /// A notice of `kind` about the file `name` in a shared folder.
    fn about(kind: EventKind, name: &str) -> Event {
        Event::new(kind).add_path(Path::new("shared").join(name))
    }

    #[test]
    fn each_notice_is_told_as_the_core_reads_it() {
        let touched = |names: &[&str]| {
            names
                .iter()
                .map(|name| Told::Touched((*name).to_owned()))
                .collect()
        };
        assert_eq!(
            reported(Ok(about(EventKind::Create(CreateKind::File), "ab.youarel"))),
            (touched(&["ab.youarel"]), false)
        );
        assert_eq!(
            reported(Ok(about(EventKind::Modify(ModifyKind::Any), "ab.youarel")
                .add_path(Path::new("shared").join("cd.youarel")))),
            (touched(&["ab.youarel", "cd.youarel"]), false),
            "every file a notice names"
        );
        assert_eq!(
            reported(Ok(about(EventKind::Remove(RemoveKind::File), "ab.youarel"))),
            (touched(&["ab.youarel"]), false),
            "a removed bundle changes the folder too"
        );
        assert_eq!(
            reported(Ok(about(EventKind::Access(AccessKind::Any), "ab.youarel"))),
            (vec![Told::Read], false)
        );
        assert_eq!(
            reported(Ok(Event::new(EventKind::Other).set_flag(Flag::Rescan))),
            (vec![Told::Rescan], false)
        );
        assert_eq!(
            reported(Ok(Event::new(EventKind::Any))),
            (vec![Told::Rescan], false),
            "a notice of no file may hide any"
        );
        assert_eq!(
            reported(Ok(about(EventKind::Remove(RemoveKind::Folder), "inside"))),
            (vec![Told::Rescan], true)
        );
        assert_eq!(
            reported(Err(notify::Error::generic("the watch failed"))),
            (vec![Told::Rescan], true)
        );
    }
}
