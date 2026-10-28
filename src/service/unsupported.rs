use std::io;

use super::{Registration, Uninstalled};

/// No service manager is supported on this system.
pub fn install(_registration: &Registration) -> io::Result<String> {
    Err(unsupported())
}

/// No service manager is supported on this system.
pub fn uninstall() -> io::Result<Uninstalled> {
    Err(unsupported())
}

fn unsupported() -> io::Error {
    io::Error::other("install is not supported on this system")
}
