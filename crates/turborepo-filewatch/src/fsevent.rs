//! Watcher implementation for Darwin's FSEvents API
//!
//! The FSEvents API provides a mechanism to notify clients about directories
//! they ought to re-scan in order to keep their internal data structures
//! up-to-date with respect to the true state of the file system. (For example,
//! when files or directories are created, modified, or removed.) It sends these
//! notifications "in bulk", possibly notifying the client of changes to several
//! directories in a single callback.
//!
//! For more information see the [FSEvents API reference][ref].
//!
//! TODO: document event translation
//!
//! [ref]: https://developer.apple.com/library/mac/documentation/Darwin/Reference/FSEvents_Ref/

#![allow(non_upper_case_globals, dead_code)]
// bitflags! with a 0 value defined triggers this clippy error,
// but we want to be able to define a value for fs::kFSEventStreamEventFlagNone
#![allow(clippy::bad_bit_mask)]

use std::{
    collections::HashMap,
    ffi::{CStr, OsStr},
    fmt,
    os::{raw, unix::ffi::OsStrExt},
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use fs::core_foundation::Boolean;
use fsevent_sys as fs;
use fsevent_sys::core_foundation as cf;
use notify::{
    Config, Error, Event, EventHandler, EventKind, RecursiveMode, Result, Watcher, WatcherKind,
    event::{CreateKind, DataChange, Flag, MetadataKind, ModifyKind, RemoveKind, RenameMode},
};

//use crate::event::*;

type Sender<T> = std::sync::mpsc::Sender<T>;

bitflags::bitflags! {
  #[repr(C)]
  struct StreamFlags: u32 {
    const NONE = fs::kFSEventStreamEventFlagNone;
    const MUST_SCAN_SUBDIRS = fs::kFSEventStreamEventFlagMustScanSubDirs;
    const USER_DROPPED = fs::kFSEventStreamEventFlagUserDropped;
    const KERNEL_DROPPED = fs::kFSEventStreamEventFlagKernelDropped;
    const IDS_WRAPPED = fs::kFSEventStreamEventFlagEventIdsWrapped;
    const HISTORY_DONE = fs::kFSEventStreamEventFlagHistoryDone;
    const ROOT_CHANGED = fs::kFSEventStreamEventFlagRootChanged;
    const MOUNT = fs::kFSEventStreamEventFlagMount;
    const UNMOUNT = fs::kFSEventStreamEventFlagUnmount;
    const ITEM_CREATED = fs::kFSEventStreamEventFlagItemCreated;
    const ITEM_REMOVED = fs::kFSEventStreamEventFlagItemRemoved;
    const INODE_META_MOD = fs::kFSEventStreamEventFlagItemInodeMetaMod;
    const ITEM_RENAMED = fs::kFSEventStreamEventFlagItemRenamed;
    const ITEM_MODIFIED = fs::kFSEventStreamEventFlagItemModified;
    const FINDER_INFO_MOD = fs::kFSEventStreamEventFlagItemFinderInfoMod;
    const ITEM_CHANGE_OWNER = fs::kFSEventStreamEventFlagItemChangeOwner;
    const ITEM_XATTR_MOD = fs::kFSEventStreamEventFlagItemXattrMod;
    const IS_FILE = fs::kFSEventStreamEventFlagItemIsFile;
    const IS_DIR = fs::kFSEventStreamEventFlagItemIsDir;
    const IS_SYMLINK = fs::kFSEventStreamEventFlagItemIsSymlink;
    const OWN_EVENT = fs::kFSEventStreamEventFlagOwnEvent;
    const IS_HARDLINK = fs::kFSEventStreamEventFlagItemIsHardlink;
    const IS_LAST_HARDLINK = fs::kFSEventStreamEventFlagItemIsLastHardlink;
    const ITEM_CLONED = fs::kFSEventStreamEventFlagItemCloned;
  }
}

/// FSEvents-based `Watcher` implementation.
///
/// # Platform-Specific Behavior
///
/// This watcher uses `FSEventStreamCreate`, which accepts multiple absolute
/// paths across filesystem devices and reports absolute paths to the callback.
///
/// # Example
///
/// ```ignore
/// use notify::{Watcher, RecursiveMode};
///
/// let (tx, rx) = std::sync::mpsc::channel();
/// let mut watcher = FsEventWatcher::new(tx, Default::default())?;
///
/// // Watch paths on the same or different filesystem devices.
/// watcher.watch("/Users/foo/project", RecursiveMode::Recursive)?;
/// watcher.watch("/Volumes/External/other", RecursiveMode::Recursive)?;
/// ```
pub struct FsEventWatcher {
    paths: cf::CFMutableArrayRef,
    since_when: fs::FSEventStreamEventId,
    latency: cf::CFTimeInterval,
    flags: fs::FSEventStreamCreateFlags,
    event_handler: Arc<Mutex<dyn EventHandler>>,
    runloop: Option<(cf::CFRunLoopRef, thread::JoinHandle<()>)>,
    /// Maps watched paths to their recursive mode flag. `true` means recursive
    /// watching is enabled for that path and all descendants.
    ///
    /// Note: The callback iterates over this map for each event to determine if
    /// the event should be handled. This is O(n) where n is the number of
    /// watched paths, which is acceptable for typical usage (1-10 paths).
    /// For watching hundreds of paths, consider using a radix trie for
    /// O(depth) lookups.
    recursive_info: HashMap<PathBuf, bool>,
    /// Last event observed by the callback, used to resume after watch changes.
    latest_event_id: Arc<AtomicU64>,
}

impl fmt::Debug for FsEventWatcher {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("FsEventWatcher")
            .field("paths", &self.paths)
            .field("since_when", &self.since_when)
            .field("latency", &self.latency)
            .field("flags", &self.flags)
            .field("event_handler", &Arc::as_ptr(&self.event_handler))
            .field("runloop", &self.runloop)
            .field("recursive_info", &self.recursive_info)
            .finish()
    }
}

// CFMutableArrayRef is a type alias to *mut libc::c_void, so FsEventWatcher is
// not Send/Sync automatically. It's Send because the pointer is not used in
// other threads.
unsafe impl Send for FsEventWatcher {}

// It's Sync because all methods that change the mutable state use `&mut self`.
unsafe impl Sync for FsEventWatcher {}

fn translate_flags(flags: StreamFlags, precise: bool) -> Vec<Event> {
    let mut evs = Vec::new();

    // «Denotes a sentinel event sent to mark the end of the "historical" events
    // sent as a result of specifying a `sinceWhen` value in the FSEvents.Create
    // call that created this event stream. After invoking the client's callback
    // with all the "historical" events that occurred before now, the client's
    // callback will be invoked with an event where the HistoryDone flag is set.
    // The client should ignore the path supplied in this callback.»
    // — https://www.mbsplugins.eu/FSEventsNextEvent.shtml
    //
    // As a result, we just stop processing here and return an empty vec, which
    // will ignore this completely and not emit any Events whatsoever.
    if flags.contains(StreamFlags::HISTORY_DONE) {
        return evs;
    }

    // FSEvents provides two possible hints as to why events were dropped,
    // however documentation on what those mean is scant, so we just pass them
    // through in the info attr field. The intent is clear enough, and the
    // additional information is provided if the user wants it.
    if flags.contains(StreamFlags::MUST_SCAN_SUBDIRS) {
        let e = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        evs.push(if flags.contains(StreamFlags::USER_DROPPED) {
            e.set_info("rescan: user dropped")
        } else if flags.contains(StreamFlags::KERNEL_DROPPED) {
            e.set_info("rescan: kernel dropped")
        } else {
            e
        });
    }

    // In imprecise mode, let's not even bother parsing the kind of the event
    // except for the above very special events.
    if !precise {
        evs.push(Event::new(EventKind::Any));
        return evs;
    }

    // This is most likely a rename or a removal. We assume rename but may want
    // to figure out if it was a removal some way later (TODO). To denote the
    // special nature of the event, we add an info string.
    if flags.contains(StreamFlags::ROOT_CHANGED) {
        evs.push(
            Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From)))
                .set_info("root changed"),
        );
    }

    // A path was mounted at the event path; we treat that as a create.
    if flags.contains(StreamFlags::MOUNT) {
        evs.push(Event::new(EventKind::Create(CreateKind::Other)).set_info("mount"));
    }

    // A path was unmounted at the event path; we treat that as a remove.
    if flags.contains(StreamFlags::UNMOUNT) {
        evs.push(Event::new(EventKind::Remove(RemoveKind::Other)).set_info("mount"));
    }

    if flags.contains(StreamFlags::ITEM_CREATED) {
        evs.push(if flags.contains(StreamFlags::IS_DIR) {
            Event::new(EventKind::Create(CreateKind::Folder))
        } else if flags.contains(StreamFlags::IS_FILE) {
            Event::new(EventKind::Create(CreateKind::File))
        } else {
            let e = Event::new(EventKind::Create(CreateKind::Other));
            if flags.contains(StreamFlags::IS_SYMLINK) {
                e.set_info("is: symlink")
            } else if flags.contains(StreamFlags::IS_HARDLINK) {
                e.set_info("is: hardlink")
            } else if flags.contains(StreamFlags::ITEM_CLONED) {
                e.set_info("is: clone")
            } else {
                Event::new(EventKind::Create(CreateKind::Any))
            }
        });
    }

    if flags.contains(StreamFlags::ITEM_REMOVED) {
        evs.push(if flags.contains(StreamFlags::IS_DIR) {
            Event::new(EventKind::Remove(RemoveKind::Folder))
        } else if flags.contains(StreamFlags::IS_FILE) {
            Event::new(EventKind::Remove(RemoveKind::File))
        } else {
            let e = Event::new(EventKind::Remove(RemoveKind::Other));
            if flags.contains(StreamFlags::IS_SYMLINK) {
                e.set_info("is: symlink")
            } else if flags.contains(StreamFlags::IS_HARDLINK) {
                e.set_info("is: hardlink")
            } else if flags.contains(StreamFlags::ITEM_CLONED) {
                e.set_info("is: clone")
            } else {
                Event::new(EventKind::Remove(RemoveKind::Any))
            }
        });
    }

    // FSEvents provides no mechanism to associate the old and new sides of a
    // rename event.
    if flags.contains(StreamFlags::ITEM_RENAMED) {
        evs.push(Event::new(EventKind::Modify(ModifyKind::Name(
            RenameMode::Any,
        ))));
    }

    // This is only described as "metadata changed", but it may be that it's
    // only emitted for some more precise subset of events... if so, will need
    // amending, but for now we have an Any-shaped bucket to put it in.
    if flags.contains(StreamFlags::INODE_META_MOD) {
        evs.push(Event::new(EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::Any,
        ))));
    }

    if flags.contains(StreamFlags::FINDER_INFO_MOD) {
        evs.push(
            Event::new(EventKind::Modify(ModifyKind::Metadata(MetadataKind::Other)))
                .set_info("meta: finder info"),
        );
    }

    if flags.contains(StreamFlags::ITEM_CHANGE_OWNER) {
        evs.push(Event::new(EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::Ownership,
        ))));
    }

    if flags.contains(StreamFlags::ITEM_XATTR_MOD) {
        evs.push(Event::new(EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::Extended,
        ))));
    }

    // This is specifically described as a data change, which we take to mean
    // is a content change.
    if flags.contains(StreamFlags::ITEM_MODIFIED) {
        evs.push(Event::new(EventKind::Modify(ModifyKind::Data(
            DataChange::Content,
        ))));
    }

    if flags.contains(StreamFlags::OWN_EVENT) {
        for ev in &mut evs {
            *ev = std::mem::take(ev).set_process_id(std::process::id());
        }
    }

    evs
}

struct StreamContextInfo {
    event_handler: Arc<Mutex<dyn EventHandler>>,
    recursive_info: HashMap<PathBuf, bool>,
    latest_event_id: Arc<AtomicU64>,
}

struct StreamOwner(fs::FSEventStreamRef);

impl StreamOwner {
    fn as_ptr(&self) -> fs::FSEventStreamRef {
        self.0
    }
}

// Safety: Apple documents Core Foundation references as safe to move between
// threads. The stream itself is used only by the spawned run-loop thread.
unsafe impl Send for StreamOwner {}

impl Drop for StreamOwner {
    fn drop(&mut self) {
        unsafe {
            fs::FSEventStreamInvalidate(self.0);
            fs::FSEventStreamRelease(self.0);
        }
    }
}

// Free the context when the stream created by `FSEventStreamCreate` is
// released.
extern "C" fn release_context(info: *const libc::c_void) {
    // Safety:
    // - The [documentation] for `FSEventStreamContext` states that `release` is
    //   only called when the stream is deallocated, so it is safe to convert `info`
    //   back into a box and drop it.
    //
    // [docs]: https://developer.apple.com/documentation/coreservices/fseventstreamcontext?language=objc
    unsafe {
        drop(Box::from_raw(
            info as *const StreamContextInfo as *mut StreamContextInfo,
        ));
    }
}

unsafe extern "C" {
    /// Indicates whether the run loop is waiting for an event.
    safe fn CFRunLoopIsWaiting(runloop: cf::CFRunLoopRef) -> cf::Boolean;
}

// CoreFoundation false value
const FALSE: Boolean = 0x0;

impl FsEventWatcher {
    fn from_event_handler(event_handler: Arc<Mutex<dyn EventHandler>>) -> Result<Self> {
        let since_when = unsafe { fs::FSEventsGetCurrentEventId() };
        Ok(FsEventWatcher {
            paths: unsafe {
                cf::CFArrayCreateMutable(cf::kCFAllocatorDefault, 0, &cf::kCFTypeArrayCallBacks)
            },
            since_when,
            latency: 0.01,
            flags: fs::kFSEventStreamCreateFlagFileEvents
                | fs::kFSEventStreamCreateFlagNoDefer
                | fs::kFSEventStreamCreateFlagWatchRoot,
            event_handler,
            runloop: None,
            recursive_info: HashMap::new(),
            latest_event_id: Arc::new(AtomicU64::new(since_when)),
        })
    }

    fn watch_inner(&mut self, path: &Path, recursive_mode: RecursiveMode) -> Result<()> {
        self.stop();
        let result = self.append_path(path, recursive_mode);
        let restart = self.restart();
        Self::finish_restart(result, restart)
    }

    fn unwatch_inner(&mut self, path: &Path) -> Result<()> {
        self.stop();
        let result = self.remove_path(path);
        let restart = self.restart();
        Self::finish_restart(result, restart)
    }

    fn finish_restart(operation: Result<()>, restart: Result<()>) -> Result<()> {
        match (operation, restart) {
            (_, Err(error)) => Err(error),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn restart(&mut self) -> Result<()> {
        if unsafe { cf::CFArrayGetCount(self.paths) } == 0 {
            Ok(())
        } else {
            self.run()
        }
    }

    #[inline]
    fn is_running(&self) -> bool {
        self.runloop.is_some()
    }

    fn stop(&mut self) {
        if !self.is_running() {
            return;
        }

        if let Some((runloop, thread_handle)) = self.runloop.take() {
            unsafe {
                let runloop = runloop as *mut raw::c_void;

                while CFRunLoopIsWaiting(runloop) == 0 {
                    thread::yield_now();
                }

                cf::CFRunLoopStop(runloop);
            }

            // Wait for the thread to shut down.
            let _ = thread_handle.join();
            let latest_event_id = self.latest_event_id.load(Ordering::Acquire);
            if latest_event_id != 0 {
                self.since_when = latest_event_id;
            }
        }
    }

    fn remove_path(&mut self, path: &Path) -> Result<()> {
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        let str_path = canonical_path
            .to_str()
            .ok_or_else(|| Error::generic("path contains invalid UTF-8"))?;
        unsafe {
            let mut err: cf::CFErrorRef = ptr::null_mut();
            let cf_path = cf::str_path_to_cfstring_ref(str_path, &mut err);
            if cf_path.is_null() {
                cf::CFRelease(err as cf::CFRef);
                return Err(Error::watch_not_found().add_path(path.into()));
            }

            let mut to_remove = Vec::new();
            for idx in 0..cf::CFArrayGetCount(self.paths) {
                let item = cf::CFArrayGetValueAtIndex(self.paths, idx);
                if cf::CFStringCompare(item, cf_path, cf::kCFCompareCaseInsensitive)
                    == cf::kCFCompareEqualTo
                {
                    to_remove.push(idx);
                }
            }

            cf::CFRelease(cf_path);

            for idx in to_remove.iter().rev() {
                cf::CFArrayRemoveValueAtIndex(self.paths, *idx);
            }
        }
        match self.recursive_info.remove(&canonical_path) {
            Some(_) => Ok(()),
            None => Err(Error::watch_not_found()),
        }
    }

    // https://github.com/thibaudgg/rb-fsevent/blob/master/ext/fsevent_watch/main.c
    //
    fn append_path(&mut self, path: &Path, recursive_mode: RecursiveMode) -> Result<()> {
        let canonical_path = path.to_path_buf().canonicalize()?;
        let str_path = canonical_path
            .to_str()
            .ok_or_else(|| Error::generic("path contains invalid UTF-8"))?;

        unsafe {
            let mut err: cf::CFErrorRef = ptr::null_mut();
            let cf_path = cf::str_path_to_cfstring_ref(str_path, &mut err);
            if cf_path.is_null() {
                // Most likely the directory was deleted, or permissions changed,
                // while the above code was running.
                cf::CFRelease(err as cf::CFRef);
                return Err(Error::path_not_found().add_path(path.into()));
            }
            cf::CFArrayAppendValue(self.paths, cf_path);
            cf::CFRelease(cf_path);
        }
        let is_recursive = matches!(recursive_mode, RecursiveMode::Recursive);
        self.recursive_info.insert(canonical_path, is_recursive);
        Ok(())
    }

    fn run(&mut self) -> Result<()> {
        if unsafe { cf::CFArrayGetCount(self.paths) } == 0 {
            // TODO: Reconstruct and add paths to error
            return Err(Error::path_not_found());
        }
        // Associate the stream context with our callback. FSEvents owns it once
        // stream creation succeeds, and `StreamOwner` releases the stream on
        // every thread startup and shutdown path.
        let stream_context_info = Box::into_raw(Box::new(StreamContextInfo {
            event_handler: self.event_handler.clone(),
            recursive_info: self.recursive_info.clone(),
            latest_event_id: self.latest_event_id.clone(),
        }));

        let stream_context = fs::FSEventStreamContext {
            version: 0,
            info: stream_context_info as *mut libc::c_void,
            retain: None,
            release: Some(release_context),
            copy_description: None,
        };

        let stream = unsafe {
            fs::FSEventStreamCreate(
                cf::kCFAllocatorDefault,
                callback,
                &stream_context,
                self.paths,
                self.since_when,
                self.latency,
                self.flags,
            )
        };
        if stream.is_null() {
            unsafe {
                drop(Box::from_raw(stream_context_info));
            }
            return Err(Error::generic("FSEventStreamCreate failed"));
        }

        // Wrapper to help send CFRef types across threads.
        struct CFSendWrapper(cf::CFRef);

        // Safety:
        // - According to the Apple documentation, it's safe to move `CFRef`s across threads.
        //   https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Multithreading/ThreadSafetySummary/ThreadSafetySummary.html
        unsafe impl Send for CFSendWrapper {}

        let stream = StreamOwner(stream);

        // channel to pass runloop around
        let (rl_tx, rl_rx) = std::sync::mpsc::channel::<Result<CFSendWrapper>>();

        let thread_handle = thread::Builder::new()
            .name("notify-rs fsevents loop".to_string())
            .spawn(move || {
                let stream_ref = stream.as_ptr();

                unsafe {
                    let cur_runloop = cf::CFRunLoopGetCurrent();

                    fs::FSEventStreamScheduleWithRunLoop(
                        stream_ref,
                        cur_runloop,
                        cf::kCFRunLoopDefaultMode,
                    );
                    if fs::FSEventStreamStart(stream_ref) == FALSE {
                        let _ = rl_tx.send(Err(Error::generic("FSEventStream failed to start")));
                        return;
                    }

                    // the calling to CFRunLoopRun will be terminated by CFRunLoopStop call in
                    // drop()
                    if rl_tx.send(Ok(CFSendWrapper(cur_runloop))).is_err() {
                        fs::FSEventStreamStop(stream_ref);
                        return;
                    }

                    cf::CFRunLoopRun();
                    fs::FSEventStreamStop(stream_ref);
                }
            })?;
        // block until runloop has been sent
        let runloop = match rl_rx.recv() {
            Ok(Ok(runloop)) => runloop,
            Ok(Err(error)) => {
                let _ = thread_handle.join();
                return Err(error);
            }
            Err(_) => {
                let _ = thread_handle.join();
                return Err(Error::generic("runloop channel disconnected"));
            }
        };
        self.runloop = Some((runloop.0, thread_handle));

        Ok(())
    }

    fn configure_raw_mode(&mut self, _config: Config, tx: Sender<Result<bool>>) {
        let _ = tx.send(Ok(false));
    }
}

extern "C" fn callback(
    stream_ref: fs::FSEventStreamRef,
    info: *mut libc::c_void,
    num_events: libc::size_t,       // size_t numEvents
    event_paths: *mut libc::c_void, // void *eventPaths
    event_flags: *const fs::FSEventStreamEventFlags, /* const FSEventStreamEventFlags
                                     * eventFlags[] */
    event_ids: *const fs::FSEventStreamEventId, // const FSEventStreamEventId eventIds[]
) {
    unsafe {
        callback_impl(
            stream_ref,
            info,
            num_events,
            event_paths,
            event_flags,
            event_ids,
        )
    }
}

/// Implementation of the FSEvents callback.
///
/// # Safety
///
/// This function is called from C code and must not panic, as unwinding across
/// FFI boundaries is undefined behavior. All error conditions are handled
/// gracefully by skipping malformed events.
unsafe fn callback_impl(
    _stream_ref: fs::FSEventStreamRef,
    info: *mut libc::c_void,
    num_events: libc::size_t,                        // size_t numEvents
    event_paths: *mut libc::c_void,                  // void *eventPaths
    event_flags: *const fs::FSEventStreamEventFlags, // const FSEventStreamEventFlags eventFlags[]
    event_ids: *const fs::FSEventStreamEventId,      // const FSEventStreamEventId eventIds[]
) {
    let event_paths = event_paths as *const *const libc::c_char;
    let info = info as *const StreamContextInfo;
    let event_handler = unsafe { &(*info).event_handler };
    let latest_event_id = unsafe { &(*info).latest_event_id };

    for p in 0..num_events {
        let event_id = unsafe { *event_ids.add(p) };
        latest_event_id.fetch_max(event_id, Ordering::Release);
        // SAFETY: The FSEvents path is a NUL-terminated byte string valid for
        // this callback. Preserve its bytes because Unix paths need not be UTF-8.
        let raw_path = unsafe { CStr::from_ptr(*event_paths.add(p)) };

        let path = PathBuf::from(OsStr::from_bytes(raw_path.to_bytes()));

        let flag = unsafe { *event_flags.add(p) };
        // Use from_bits_truncate to handle unknown flags gracefully instead of
        // panicking. Unknown flags are ignored, which is safe as they represent
        // future FSEvents features.
        let flag = StreamFlags::from_bits_truncate(flag);

        // Note: This is O(n) where n is the number of watched paths.
        // For typical usage (1-10 paths), this is acceptable.
        // For hundreds of paths, consider using a radix trie.
        let mut handle_event = false;
        for (p, r) in unsafe { &(*info).recursive_info } {
            if path.starts_with(p) {
                if *r || &path == p {
                    handle_event = true;
                    break;
                } else if let Some(parent_path) = path.parent()
                    && parent_path == p
                {
                    handle_event = true;
                    break;
                }
            }
        }

        if !handle_event {
            continue;
        }

        for ev in translate_flags(flag, true).into_iter() {
            // TODO: precise
            let ev = ev.add_path(path.clone());
            let mut event_handler = event_handler
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            event_handler.handle_event(Ok(ev));
        }
    }
}

impl Watcher for FsEventWatcher {
    /// Create a new watcher.
    fn new<F: EventHandler>(event_handler: F, _config: Config) -> Result<Self> {
        Self::from_event_handler(Arc::new(Mutex::new(event_handler)))
    }

    fn watch(&mut self, path: &Path, recursive_mode: RecursiveMode) -> Result<()> {
        self.watch_inner(path, recursive_mode)
    }

    fn unwatch(&mut self, path: &Path) -> Result<()> {
        self.unwatch_inner(path)
    }

    fn configure(&mut self, config: Config) -> Result<bool> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.configure_raw_mode(config, tx);
        rx.recv()
            .map_err(|err| Error::generic(&format!("internal channel disconnect: {err:?}")))?
    }

    fn kind() -> WatcherKind {
        WatcherKind::Fsevent
    }
}

impl Drop for FsEventWatcher {
    fn drop(&mut self) {
        self.stop();
        unsafe {
            cf::CFRelease(self.paths);
        }
    }
}

#[test]
fn test_fsevent_watcher_drop() {
    use std::time::Duration;

    use super::*;

    let dir = tempfile::tempdir().unwrap();

    let (tx, rx) = std::sync::mpsc::channel();

    {
        let mut watcher = FsEventWatcher::new(tx, Default::default()).unwrap();
        watcher.watch(dir.path(), RecursiveMode::Recursive).unwrap();
        thread::sleep(Duration::from_millis(2000));
        //println!("is running -> {}", watcher.is_running());

        thread::sleep(Duration::from_millis(1000));
        watcher.unwatch(dir.path()).unwrap();
        //println!("is running -> {}", watcher.is_running());
    }

    thread::sleep(Duration::from_millis(1000));

    for res in rx {
        let e = res.unwrap();
        println!("debug => {:?} {:?}", e.kind, e.paths);
    }

    println!("in test: {} works", file!());
}

#[test]
fn test_steam_context_info_send_and_sync() {
    fn check_send<T: Send + Sync>() {}
    check_send::<StreamContextInfo>();
}

#[test]
fn test_fsevent_watches_multiple_paths() {
    use std::time::{Duration, Instant};

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let first = root.join("first");
    let second = root.join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = FsEventWatcher::new(tx, Default::default()).unwrap();
    watcher.watch(&first, RecursiveMode::Recursive).unwrap();

    let first_file = first.join("first.txt");
    let second_file = second.join("second.txt");
    // Create an event immediately before adding a second path. Restarting the
    // stream must replay this event from the previous watermark.
    std::fs::write(&first_file, b"first").unwrap();
    watcher.watch(&second, RecursiveMode::Recursive).unwrap();
    std::fs::write(&second_file, b"second").unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_first = false;
    let mut saw_second = false;
    while Instant::now() < deadline && !(saw_first && saw_second) {
        if let Ok(Ok(event)) = rx.recv_timeout(Duration::from_millis(100)) {
            saw_first |= event.paths.contains(&first_file);
            saw_second |= event.paths.contains(&second_file);
        }
    }

    assert!(saw_first, "did not receive an event for {first_file:?}");
    assert!(saw_second, "did not receive an event for {second_file:?}");

    watcher.unwatch(&first).unwrap();
    assert_eq!(unsafe { cf::CFArrayGetCount(watcher.paths) }, 1);
    while rx.try_recv().is_ok() {}

    let remaining_file = second.join("remaining.txt");
    std::fs::write(&remaining_file, b"remaining").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(Ok(event)) = rx.recv_timeout(Duration::from_millis(100))
            && event.paths.contains(&remaining_file)
        {
            return;
        }
    }
    panic!("remaining path stopped emitting events after unwatching the first path");
}

/// A temporary RAM disk volume for testing FSEvents on non-root filesystems.
///
/// Creates a 10MB APFS-formatted RAM disk that is automatically ejected when
/// dropped. The disk identifier (e.g., `/dev/disk5`) is stored to ensure
/// reliable cleanup even if the volume path becomes unavailable.
///
/// Returns `None` if creation fails (e.g., due to permission issues with
/// `hdiutil`).
#[cfg(test)]
struct TestVolume {
    /// The mounted volume path (e.g., `/Volumes/TurboXXXXXX`)
    path: PathBuf,
    /// The disk identifier (e.g., `/dev/disk5`) for guaranteed cleanup
    disk: String,
}

/// RAM disk size: 20480 sectors * 512 bytes = 10MB
#[cfg(test)]
const TEST_RAMDISK_SECTORS: u32 = 20480;

#[cfg(test)]
impl TestVolume {
    fn new() -> Option<Self> {
        use std::process::Command;

        use tempfile::TempDir;

        let temp = TempDir::with_prefix("Turbo").ok()?;
        let name = temp.path().file_name()?.to_str()?;

        // Create RAM disk
        let output = Command::new("hdiutil")
            .args([
                "attach",
                "-nomount",
                &format!("ram://{}", TEST_RAMDISK_SECTORS),
            ])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let disk = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Format the disk - if this fails, we need to detach the disk we created
        let status = Command::new("diskutil")
            .args(["erasevolume", "APFS", name, &disk])
            .status()
            .ok()?;

        if !status.success() {
            // Try to detach the disk if formatting failed
            let _ = Command::new("hdiutil")
                .args(["detach", "-force", &disk])
                .status();
            return None;
        }

        let path = PathBuf::from(format!("/Volumes/{}", name));
        if path.exists() {
            Some(Self { path, disk })
        } else {
            // Volume doesn't exist - clean up the disk
            let _ = Command::new("hdiutil")
                .args(["detach", "-force", &disk])
                .status();
            None
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
impl Drop for TestVolume {
    fn drop(&mut self) {
        use std::process::Command;

        // Best effort cleanup - try eject first (cleaner), then force detach
        // Use to_string_lossy() to avoid panic on non-UTF8 paths in Drop
        let _ = Command::new("diskutil")
            .args(["eject", &self.path.to_string_lossy()])
            .status();

        // Force detach by disk identifier as fallback - this always works
        let _ = Command::new("hdiutil")
            .args(["detach", "-force", &self.disk])
            .status();
    }
}

/// Test that file paths are reported correctly on non-root volumes.
///
/// This creates a temporary RAM disk to ensure we're testing on a non-root
/// volume to ensure the absolute-path stream reports the mounted path.
#[test]
fn test_fsevent_reports_correct_absolute_paths() {
    use std::time::{Duration, Instant};

    // Create a RAM disk to guarantee we're testing on a non-root volume
    let volume = match TestVolume::new() {
        Some(v) => v,
        None => {
            eprintln!(
                "Skipping test: unable to create RAM disk (may require elevated permissions)"
            );
            return;
        }
    };

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = FsEventWatcher::new(tx, Default::default()).unwrap();
    let local = tempfile::tempdir().unwrap();
    watcher
        .watch(local.path(), RecursiveMode::Recursive)
        .unwrap();
    watcher
        .watch(volume.path(), RecursiveMode::Recursive)
        .unwrap();

    thread::sleep(Duration::from_millis(1000)); // FSEvents init time

    let test_file = volume.path().join("test_file.txt");
    std::fs::write(&test_file, b"test").unwrap();

    // Wait for event with correct path
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut received = Vec::new();
    while Instant::now() < deadline {
        if let Ok(Ok(event)) = rx.recv_timeout(Duration::from_millis(100)) {
            received.extend(event.paths.clone());
            if event.paths.contains(&test_file) {
                return; // Success
            }
        }
    }

    panic!(
        "Expected event for {:?}, received: {:?}",
        test_file, received
    );
}
