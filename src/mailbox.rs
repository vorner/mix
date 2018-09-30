use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;

use failure::Error;
use flate2::read::GzDecoder;
use lazy_static::lazy_static;
use log::{debug, error, info, trace};
use parking_lot::Mutex;
use walkdir::{DirEntry, WalkDir};

use crate::config::Cfg;

lazy_static! {
    pub(crate) static ref MAILBOXES: Mutex<HashMap<String, Arc<Mailbox>>> = Mutex::default();
}

const GZIP_MAGIC: &[u8] = &[0x1F, 0x8B];
const MBOX_MAGIC: &[u8] = b"From ";
const MDIR_SUBDIRS: &[&str] = &["cur", "new", "tmp"];

#[derive(Debug)]
enum MboxType {
    Plain,
    Gzip,
    Dir,
}

impl MboxType {
    fn guess(entry: &DirEntry) -> Result<Option<Self>, Error> {
        if entry.file_type().is_file() {
            // It is a file. So try opening it and look inside.
            let mut f = File::open(entry.path())?;
            let mut beginning = [0u8; 5];
            f.read_exact(&mut beginning)?;
            trace!("{:?} starts with {:?}", entry.path(), beginning);
            if beginning == MBOX_MAGIC {
                return Ok(Some(MboxType::Plain));
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
                    return Ok(Some(MboxType::Gzip));
                }
            }
        } else if entry.file_type().is_dir() {
            // Not every dir is a maildir ‒ maildirs have specific subdirs in them.
            let is_mdir = MDIR_SUBDIRS
                .iter()
                .all(|sub| entry.path().join(sub).is_dir());
            if is_mdir {
                return Ok(Some(MboxType::Dir));
            }
        }
        Ok(None)
    }
}

#[derive(Debug)]
crate struct Mailbox {
    path: PathBuf,
    name: String,
    tp: MboxType,
}

impl Mailbox {
    fn detect(entry: &DirEntry) -> Result<Option<Self>, Error> {
        if let Some(mt) = MboxType::guess(entry)? {
            let name = entry
                .path()
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<???>".to_owned());
            Ok(Some(Mailbox {
                path: entry.path().to_owned(),
                name,
                tp: mt,
            }))
        } else {
            Ok(None)
        }
    }
    crate fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug)]
crate enum Notification {
    Mailbox(Arc<Mailbox>),
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

crate fn initial_scan(cfg: &Cfg) {
    let mut dedup = HashSet::new();
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
                        let mbox = Arc::new(mbox);
                        let name = mbox.name().to_owned();
                        assert!(MAILBOXES.lock().insert(name, Arc::clone(&mbox)).is_none());
                        Notification::send(Notification::Mailbox(mbox));
                        assert!(dedup.insert(entry.into_path()));
                    }
                }
            }
        }
    }
}
