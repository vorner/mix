use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

use config::{Config, File};
use failure::Error;
use log::{debug, trace};
use serde_derive::Deserialize;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct CmdLine {
    #[structopt(parse(from_os_str))]
    config: PathBuf,
}

fn default_socket() -> PathBuf {
    let home = env::var("HOME")
        .unwrap_or_else(|_| String::new());
    PathBuf::from(home).join("mix-socket")
}

#[derive(Debug, Deserialize)]
crate struct StorageMeta {
    crate shortcut: Option<char>,
    #[serde(default)]
    crate prio: u8,
}

#[derive(Debug, Deserialize)]
crate struct Storage {
    crate search: Vec<PathBuf>,
    #[serde(default)]
    crate meta: HashMap<PathBuf, StorageMeta>,
}

#[derive(Debug, Deserialize)]
crate struct Cfg {
    #[serde(default = "default_socket")]
    crate socket: PathBuf,
    crate storage: Storage,
    #[serde(default)]
    crate scripts: Vec<PathBuf>,
}

crate fn load() -> Result<Cfg, Error> {
    trace!("Loading");
    let cmd_line = CmdLine::from_args();
    debug!("Command line: {:?}", cmd_line);

    let mut cfg = Config::new();
    cfg.merge(File::from(cmd_line.config))?;
    let cfg = cfg.try_into()?;
    debug!("Configuration: {:?}", cfg);
    Ok(cfg)
}
