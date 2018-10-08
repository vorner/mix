#![feature(crate_visibility_modifier, nll)]
#![forbid(unsafe_code)]

use failure::{Error, ResultExt};
use log::{debug, error};

mod config;
mod mailbox;

fn run() -> Result<(), Error> {
    let cfg = config::load()
        .context("Failed to load configuration")?;
    let work_queue = mailbox::initial_scan(&cfg)?;
    debug!("Mailboxes: {:?}", *mailbox::MAILBOXES.lock());
    debug!("Initial work queue: {:?}", work_queue);
    Ok(())
}

fn main() {
    env_logger::init();
    if let Err(e) = run() {
        error!("{}", e);
        for cause in e.iter_causes() {
            error!("Because: {}", cause);
        }
        debug!("Backtrace: {}", e.backtrace());
    }
}
