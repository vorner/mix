use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use failure::{Error, ResultExt};
use flate2::read::GzDecoder;
use log::{debug, error, info, trace};
use once_cell::sync_lazy;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use rlua::{Lua, Function, UserData, UserDataMethods, Table};
use walkdir::{DirEntry, WalkDir};

mod mbox;
mod mdir;
mod task;

use crate::config::Cfg;
use self::mbox::Mbox;
use self::mdir::Mdir;
use self::task::{Queue, Task};

crate static MAILBOXES: Lazy<Mutex<HashMap<String, Arc<Mailbox>>>> = sync_lazy!(Mutex::default());

const GZIP_MAGIC: &[u8] = &[0x1F, 0x8B];
const MBOX_MAGIC: &[u8] = b"From ";
const MDIR_SUBDIRS: &[&str] = &["cur", "new", "tmp"];

const CONFIG_CBACKS: &str = "config-cbacks";

#[derive(Clone, Debug)]
enum Type {
    Plain,
    Gzip,
    Dir,
}

impl Type {
    fn guess(entry: &DirEntry) -> Result<Option<Self>, Error> {
        if entry.file_type().is_file() {
            // It is a file. So try opening it and look inside.
            let mut f = File::open(entry.path())?;
            let mut beginning = [0u8; 5];
            f.read_exact(&mut beginning)?;
            trace!("{:?} starts with {:?}", entry.path(), beginning);
            if beginning == MBOX_MAGIC {
                return Ok(Some(Type::Plain));
            }

            // OK, if it's not a mailbox, it still can be a gzipped mailbox. Look if it starts with
            // gzip magic.
            //
            // We check 2 bytes only, but the gzip header is longer than that ‒ so the read for 5
            // bytes must not have failed.
            if &beginning[..2] == GZIP_MAGIC {
                // Try to read decompressed beginning of the file
                f.seek(SeekFrom::Start(0))?;
                let mut gz = GzDecoder::new(f);
                gz.read_exact(&mut beginning)?;

                if beginning == MBOX_MAGIC {
                    return Ok(Some(Type::Gzip));
                }
            }
        } else if entry.file_type().is_dir() {
            // Not every dir is a maildir ‒ maildirs have specific subdirs in them.
            let is_mdir = MDIR_SUBDIRS
                .iter()
                .all(|sub| entry.path().join(sub).is_dir());
            if is_mdir {
                return Ok(Some(Type::Dir));
            }
        }
        Ok(None)
    }
}

#[derive(Clone, Debug)]
enum Cache {
    Mbox(Mbox),
    Mdir(Mdir),
}

#[derive(Clone, Debug)]
crate struct Mailbox {
    path: PathBuf,
    name: String,
    tp: Type,
    cache: Cache,
    prio: usize,
    shortcut: Option<char>,
}

impl Mailbox {
    fn detect(entry: &DirEntry) -> Result<Option<Self>, Error> {
        if let Some(mt) = Type::guess(entry)? {
            let name = entry
                .path()
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<???>".to_owned());
            let cache = match mt {
                Type::Gzip | Type::Plain => Cache::Mbox(Mbox::default()),
                Type::Dir => Cache::Mdir(Mdir::default()),
            };
            Ok(Some(Mailbox {
                path: entry.path().to_owned(),
                name,
                tp: mt,
                cache,
                prio: 0,
                shortcut: None,
            }))
        } else {
            Ok(None)
        }
    }
    crate fn name(&self) -> &str {
        &self.name
    }
}

impl UserData for Mailbox {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("name", |_, this, ()| Ok(this.name().to_owned()));
        methods.add_method("path", |lua: &_, this, ()| {
            let s = lua.create_string(this.path.as_os_str().as_bytes())?;
            Ok(s)
        });
        methods.add_method_mut("set_name", |_, this, name| {
            this.name = name;
            Ok(())
        });
        methods.add_method_mut("set_prio", |_, this, prio| {
            this.prio = prio;
            Ok(())
        });
        methods.add_method_mut("set_shortcut", |_, this, sc: String| {
            this.shortcut = sc.chars().nth(0);
            Ok(())
        });
    }
}

#[derive(Debug)]
crate enum Notification {
    MailboxAppeared(Arc<Mailbox>),
    MailboxContent(Arc<Mailbox>),
}

impl Notification {
    crate fn send(notification: Notification) {
        info!("{:?}", notification);
    }
}

fn scan_cutoff(dedup: &HashSet<PathBuf>, entry: &DirEntry) -> bool {
    let path = entry.path();
    // Direct duplicate
    if dedup.contains(path) {
        return true;
    }

    // A subdirectory owned by some already scanned maildir (eg. "cur", "new" or "tmp")
    if let (Some(parent), Some(last)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) {
        entry.file_type().is_dir() && MDIR_SUBDIRS.contains(&last) && dedup.contains(parent)
    } else {
        false
    }
}

fn lua_load<P: AsRef<Path>>(lua: &Lua, script: P) -> Result<(), Error> {
    debug!("Running lua script from {}", script.as_ref().display());
    let mut f = File::open(&script)?;
    // TODO: Once lua supports non-utf8 stuff, use Vec<u8>
    let mut code = Vec::new();
    f.read_to_end(&mut code)?;
    lua.exec(&code, Some(&script.as_ref().to_string_lossy())).map_err(Error::from)
}

fn configure_mbox(lua: &Lua, mbox: Mailbox) -> Result<Mailbox, Error> {
    let cbacks = lua.named_registry_value::<Table>(CONFIG_CBACKS)?;
    let handle = lua.create_userdata(mbox)?;

    for cback in cbacks.sequence_values::<Function>() {
        let cback = cback?;
        cback.call(handle.clone())?;
    }

    let result = handle.borrow::<Mailbox>()?.clone();
    Ok(result)
}

crate fn initial_scan(cfg: &Cfg) -> Result<Queue, Error> {
    let lua = Lua::new();

    trace!("Preparing configuration lua instance");
    // Set up functions the scripts can call
    lua.set_named_registry_value(CONFIG_CBACKS, lua.create_table()?)?;
    // This'll allow them to register config callbacks
    lua.globals().set("register_config", lua.create_function(|lua, c: Function| {
        let cbacks = lua.named_registry_value::<Table>(CONFIG_CBACKS)?;
        let len = cbacks.raw_len();
        cbacks.raw_set(len + 1, c)
    })?)?;

    for script in &cfg.scripts {
        lua_load(&lua, script)
            .with_context(|_| format!("Failed to load lua script {}", script.display()))?;
    }

    let mut dedup = HashSet::new();
    let mut queue = Queue::new();

    for path in &cfg.storage.search {
        let path_str = path.display();
        debug!("Looking for maildirs in {:?}", path_str);
        let mut walkdir = WalkDir::new(path)
            .follow_links(true)
            .into_iter();
        loop {
            match walkdir.next() {
                None => break,
                Some(Err(e)) => {
                    error!("Scanning for mailboxes in {}: {}", path_str, e);
                }
                Some(Ok(ref entry)) if scan_cutoff(&dedup, entry) => {
                    trace!("Not descending into {:?}", entry.path());
                    walkdir.skip_current_dir();
                }
                Some(Ok(entry)) => match Mailbox::detect(&entry) {
                    Err(e) => {
                        error!("Detecting a mailbox in {}: {}", entry.path().display(), e);
                    }
                    Ok(None) => trace!("No mailbox found in {}", entry.path().display()),
                    Ok(Some(mbox)) => {
                        let mbox = configure_mbox(&lua, mbox)
                            .with_context(|_| {
                                format!("Failed to configure mbox {}", entry.path().display())
                            })?;
                        let mbox = Arc::new(mbox);
                        let name = mbox.name().to_owned();
                        assert!(MAILBOXES.lock().insert(name, Arc::clone(&mbox)).is_none());
                        queue.push(Task::rescan(Arc::clone(&mbox)));
                        Notification::send(Notification::MailboxAppeared(mbox));
                        assert!(dedup.insert(entry.into_path()));
                    }
                }
            }
        }
    }

    Ok(queue)
}
