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
    ffi::{CStr, CString},
    fmt,
    io::ErrorKind,
    os::{raw, unix::prelude::MetadataExt},
    path::{Path, PathBuf},
    ptr,
    sync::{Arc, Mutex},
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

/// Encapsulates device information and path transformation logic.
///
/// This type handles the bidirectional conversion between absolute filesystem
/// paths and the device-relative paths used by FSEvents when watching non-root
/// volumes.
///
/// # Path Transformation Contract
///
/// When registering a watch path:
/// - `to_device_relative()` strips the mount point prefix and prepends "/"
///
/// When receiving events in the callback:
/// - `to_absolute()` joins the mount point with the device-relative path
///
/// This symmetry must be maintained for correct path reporting.
#[derive(Debug, Clone)]
struct DeviceContext {
    /// The device ID from `stat.st_dev`
    device_id: i32,
    /// The effective mount point of the device (e.g., "/", "/Volumes/Data")
    mount_point: PathBuf,
}

impl DeviceContext {
    /// Create a new DeviceContext for the given path.
    fn new(path: &Path) -> Result<Self> {
        let metadata = std::fs::symlink_metadata(path).map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                Error::path_not_found().add_path(path.into())
            } else {
                Error::io(e)
            }
        })?;
        let device_id = metadata.dev() as i32;
        let canonical_path = path.to_path_buf().canonicalize()?;
        let mount_point = get_mount_point(&canonical_path)?;

        Ok(Self {
            device_id,
            mount_point,
        })
    }

    /// Convert an absolute path to a device-relative path for FSEvents
    /// registration.
    ///
    /// Returns the path as a string suitable for
    /// `FSEventStreamCreateRelativeToDevice`.
    fn to_device_relative(&self, absolute_path: &Path) -> Result<String> {
        let relative_path = absolute_path.strip_prefix(&self.mount_point).map_err(|_| {
            Error::generic(&format!(
                "path {:?} is not under device mount point {:?}",
                absolute_path, self.mount_point
            ))
        })?;

        let relative_str = relative_path
            .to_str()
            .ok_or_else(|| Error::generic("path contains invalid UTF-8"))?;

        Ok(format!("/{}", relative_str))
    }

    /// Convert a device-relative path from FSEvents back to an absolute path.
    ///
    /// This is the inverse of `to_device_relative()`.
    fn to_absolute(&self, device_relative: &str) -> PathBuf {
        self.mount_point.join(device_relative)
    }
}

/// FSEvents-based `Watcher` implementation.
///
/// # Platform-Specific Behavior
///
/// This watcher uses `FSEventStreamCreateRelativeToDevice` which has the
/// following limitations:
///
/// - **Single device only**: All watched paths must reside on the same
///   filesystem device. Attempting to watch paths on different devices will
///   return an error with the message "cannot watch multiple devices".
///
/// - **Path handling**: Paths are converted to device-relative format for
///   FSEvents, then converted back to absolute paths when events are reported.
///   This ensures correct path reporting on non-root volumes (e.g.,
///   `/Volumes/External`).
///
/// # Example
///
/// ```ignore
/// use notify::{Watcher, RecursiveMode};
///
/// let (tx, rx) = std::sync::mpsc::channel();
/// let mut watcher = FsEventWatcher::new(tx, Default::default())?;
///
/// // Watch a path - all subsequent watches must be on the same device
/// watcher.watch("/Users/foo/project", RecursiveMode::Recursive)?;
///
/// // This would fail if /Volumes/External is a different device:
/// // watcher.watch("/Volumes/External/other", RecursiveMode::Recursive)?;
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
    /// Device context for path transformation. Set when the first path is
    /// watched. All subsequent paths must be on the same device.
    device_context: Option<DeviceContext>,
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
    /// Device context for converting device-relative paths back to absolute
    /// paths.
    device_context: DeviceContext,
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

/// Get the effective mount point for path manipulation purposes.
///
/// This uses the statfs system call to get filesystem information.
/// If the reported mount point is not a prefix of the path (which can happen
/// on macOS with APFS firmlinks, e.g., `/private/var` reports mount point
/// `/System/Volumes/Data` but the path doesn't have that prefix), we fall
/// back to `/` as the effective mount point.
fn get_mount_point(path: &Path) -> Result<PathBuf> {
    let c_path = CString::new(
        path.to_str()
            .ok_or_else(|| Error::generic("path contains invalid UTF-8"))?,
    )
    .map_err(|_| Error::generic("path contains null byte"))?;

    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };

    let result = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) };

    if result != 0 {
        return Err(Error::io(std::io::Error::last_os_error()));
    }

    let mount_point = unsafe {
        CStr::from_ptr(stat.f_mntonname.as_ptr())
            .to_str()
            .map_err(|_| Error::generic("mount point contains invalid UTF-8"))?
    };

    // If the path doesn't start with the reported mount point, it means
    // the mount point is virtualized (e.g., via APFS firmlinks). In this
    // case, use "/" as the effective mount point for path manipulation.
    if !path.starts_with(mount_point) {
        return Ok(PathBuf::from("/"));
    }

    Ok(PathBuf::from(mount_point))
}

impl FsEventWatcher {
    fn from_event_handler(event_handler: Arc<Mutex<dyn EventHandler>>) -> Result<Self> {
        Ok(FsEventWatcher {
            paths: unsafe {
                cf::CFArrayCreateMutable(cf::kCFAllocatorDefault, 0, &cf::kCFTypeArrayCallBacks)
            },
            since_when: fs::kFSEventStreamEventIdSinceNow,
            latency: 0.01,
            flags: fs::kFSEventStreamCreateFlagFileEvents
                | fs::kFSEventStreamCreateFlagNoDefer
                | fs::kFSEventStreamCreateFlagWatchRoot,
            event_handler,
            runloop: None,
            recursive_info: HashMap::new(),
            device_context: None,
        })
    }

    fn watch_inner(&mut self, path: &Path, recursive_mode: RecursiveMode) -> Result<()> {
        self.stop();
        let result = self.append_path(path, recursive_mode);
        // ignore return error: may be empty path list
        let _ = self.run();
        result
    }

    fn unwatch_inner(&mut self, path: &Path) -> Result<()> {
        self.stop();
        let result = self.remove_path(path);
        // ignore return error: may be empty path list
        let _ = self.run();
        result
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
            thread_handle.join().expect("thread to shut down");
        }
    }

    fn remove_path(&mut self, path: &Path) -> Result<()> {
        let str_path = path
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
        let p = if let Ok(canonicalized_path) = path.canonicalize() {
            canonicalized_path
        } else {
            path.to_owned()
        };
        match self.recursive_info.remove(&p) {
            Some(_) => Ok(()),
            None => Err(Error::watch_not_found()),
        }
    }

    // https://github.com/thibaudgg/rb-fsevent/blob/master/ext/fsevent_watch/main.c
    //
    // Path handling contract:
    // 1. Paths are canonicalized and made relative to device mount point for
    //    FSEvents
    // 2. FSEvents returns device-relative paths in callbacks
    // 3. callback_impl uses DeviceContext::to_absolute() to reconstruct absolute
    //    paths
    //
    // This symmetry is enforced by the DeviceContext type.
    fn append_path(&mut self, path: &Path, recursive_mode: RecursiveMode) -> Result<()> {
        let canonical_path = path.to_path_buf().canonicalize()?;

        // Initialize or validate device context
        let device_context = if let Some(ref ctx) = self.device_context {
            // Verify we're on the same device
            let metadata = std::fs::symlink_metadata(path).map_err(|e| {
                if e.kind() == ErrorKind::NotFound {
                    Error::path_not_found().add_path(path.into())
                } else {
                    Error::io(e)
                }
            })?;
            let device_id = metadata.dev() as i32;
            if ctx.device_id != device_id {
                return Err(Error::generic("cannot watch multiple devices"));
            }
            ctx
        } else {
            // First path - create device context
            let ctx = DeviceContext::new(path)?;
            self.device_context = Some(ctx);
            self.device_context.as_ref().unwrap()
        };

        // Use DeviceContext to convert path to device-relative format
        let str_path = device_context.to_device_relative(&canonical_path)?;

        unsafe {
            let mut err: cf::CFErrorRef = ptr::null_mut();
            let cf_path = cf::str_path_to_cfstring_ref(&str_path, &mut err);
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
        let device_context = self
            .device_context
            .clone()
            .ok_or_else(|| Error::generic("no device context set for stream"))?;

        // We need to associate the stream context with our callback in order to
        // propagate events to the rest of the system. This will be owned by the
        // stream, and will be freed when the stream is closed. This means we
        // will leak the context if we panic before reaching
        // `FSEventStreamRelease`.
        let stream_context_info = Box::into_raw(Box::new(StreamContextInfo {
            event_handler: self.event_handler.clone(),
            recursive_info: self.recursive_info.clone(),
            device_context: device_context.clone(),
        }));

        let stream_context = fs::FSEventStreamContext {
            version: 0,
            info: stream_context_info as *mut libc::c_void,
            retain: None,
            release: Some(release_context),
            copy_description: None,
        };

        let stream = unsafe {
            fs::FSEventStreamCreateRelativeToDevice(
                cf::kCFAllocatorDefault,
                callback,
                &stream_context,
                device_context.device_id,
                self.paths,
                self.since_when,
                self.latency,
                self.flags,
            )
        };

        // Wrapper to help send CFRef types across threads.
        struct CFSendWrapper(cf::CFRef);

        // Safety:
        // - According to the Apple documentation, it's safe to move `CFRef`s across threads.
        //   https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Multithreading/ThreadSafetySummary/ThreadSafetySummary.html
        unsafe impl Send for CFSendWrapper {}

        // move into thread
        let stream = CFSendWrapper(stream);

        // channel to pass runloop around
        let (rl_tx, rl_rx) = std::sync::mpsc::channel();

        let thread_handle = thread::Builder::new()
            .name("notify-rs fsevents loop".to_string())
            .spawn(move || {
                let _ = &stream;
                let stream = stream.0;

                unsafe {
                    let cur_runloop = cf::CFRunLoopGetCurrent();

                    fs::FSEventStreamScheduleWithRunLoop(
                        stream,
                        cur_runloop,
                        cf::kCFRunLoopDefaultMode,
                    );
                    if fs::FSEventStreamStart(stream) == FALSE {
                        panic!("FSEventStream failed to start");
                    }

                    // the calling to CFRunLoopRun will be terminated by CFRunLoopStop call in
                    // drop()
                    rl_tx
                        .send(CFSendWrapper(cur_runloop))
                        .expect("Unable to send runloop to watcher");

                    cf::CFRunLoopRun();
                    fs::FSEventStreamStop(stream);
                    fs::FSEventStreamInvalidate(stream);
                    fs::FSEventStreamRelease(stream);
                }
            })?;
        // block until runloop has been sent
        self.runloop = Some((rl_rx.recv().unwrap().0, thread_handle));

        Ok(())
    }

    fn configure_raw_mode(&mut self, _config: Config, tx: Sender<Result<bool>>) {
        tx.send(Ok(false))
            .expect("configuration channel disconnect");
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
    _event_ids: *const fs::FSEventStreamEventId,     // const FSEventStreamEventId eventIds[]
) {
    let event_paths = event_paths as *const *const libc::c_char;
    let info = info as *const StreamContextInfo;
    let event_handler = unsafe { &(*info).event_handler };
    let device_context = unsafe { &(*info).device_context };

    for p in 0..num_events {
        // SAFETY: We must not panic in this extern "C" callback.
        // Handle invalid UTF-8 gracefully by skipping the event.
        let raw_path = match unsafe { CStr::from_ptr(*event_paths.add(p)) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                // Skip events with non-UTF8 paths rather than panic.
                // This is rare but possible with malformed filesystem entries.
                continue;
            }
        };

        // Use DeviceContext to convert device-relative path back to absolute
        let path = device_context.to_absolute(raw_path);

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
            let mut event_handler = event_handler.lock().expect("lock not to be poisoned");
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
/// volume where FSEventStreamCreateRelativeToDevice returns device-relative
/// paths that must be correctly resolved to absolute paths.
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
