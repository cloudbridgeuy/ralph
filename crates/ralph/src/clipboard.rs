use ralph_core::attachment::{file_name, parse_image, Image};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SCRIPT_LINES: [&str; 6] = [
    "on run argv",
    "set f to open for access (POSIX file (item 1 of argv)) with write permission",
    "set eof f to 0",
    "write (the clipboard as «class PNGf») to f",
    "close access f",
    "end run",
];

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

pub fn save_image(dir: &Path) -> io::Result<Option<(Image, PathBuf)>> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".clipboard-{}.tmp", std::process::id()));
    let mut command = Command::new("osascript");
    for line in SCRIPT_LINES.iter() {
        command.arg("-e").arg(line);
    }
    let status = command
        .arg(&tmp)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        remove_if_present(&tmp)?;
        return Ok(None);
    }
    let image = match parse_image(fs::read(&tmp)?) {
        Ok(image) => image,
        Err(err) => {
            remove_if_present(&tmp)?;
            return Err(io::Error::new(io::ErrorKind::InvalidData, err));
        }
    };
    let path = dir.join(file_name(&image));
    fs::rename(&tmp, &path)?;
    Ok(Some((image, path)))
}
