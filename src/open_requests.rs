use crate::{Source, opened_source};
use anyhow::{Context, Result};
use std::{
    fs,
    io::{Read as _, Write as _},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_REQUEST: u64 = 256 * 1024;

pub fn forward(directory: &Path, sources: &[Source]) -> Result<()> {
    let id = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let staging = directory.join(format!("{id}.tmp"));
    let request = directory.join(format!("{id}.open"));
    let mut body = Vec::new();
    for source in sources {
        let text = match source {
            Source::Magnet(link) => link.clone(),
            Source::File(path) => url::Url::from_file_path(std::path::absolute(path)?)
                .map_err(|()| anyhow::anyhow!("Cannot forward {}", path.display()))?
                .into(),
        };
        body.extend(text.as_bytes());
        body.push(0);
    }
    anyhow::ensure!(
        body.len() as u64 <= MAX_REQUEST,
        "Open request is too large"
    );
    let write = || -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        file.write_all(&body)?;
        drop(file);
        fs::rename(&staging, &request)?;
        Ok(())
    };
    if let Err(error) = write() {
        let _ = fs::remove_file(&staging);
        return Err(error.context("Forward to the running Fero window"));
    }
    Ok(())
}

pub fn receive(directory: &Path, opened: &Sender<Source>, activate: &AtomicBool) -> Result<()> {
    for entry in directory.read_dir()? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "open")
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        let mut body = String::new();
        let read = fs::File::open(&path)?
            .take(MAX_REQUEST + 1)
            .read_to_string(&mut body);
        if read.is_err() || body.len() as u64 > MAX_REQUEST {
            fs::remove_file(path)?;
            continue;
        }
        for source in body
            .split_terminator('\0')
            .filter_map(|text| opened_source(text.into()))
        {
            opened
                .send(source)
                .context("Fero’s desktop window is unavailable")?;
        }
        fs::remove_file(path)?;
        activate.store(true, Ordering::Release);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn running_instance_receives_existing_open_arguments() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("test torrent #1.torrent");
        let (opened, received) = mpsc::channel();
        let activate = AtomicBool::new(false);
        forward(directory.path(), &[Source::File(path.clone())])?;
        receive(directory.path(), &opened, &activate)?;
        assert!(activate.swap(false, Ordering::AcqRel));
        assert!(matches!(received.try_recv()?, Source::File(file) if file == path));
        receive(directory.path(), &opened, &activate)?;
        assert!(!activate.load(Ordering::Acquire));
        assert!(received.try_recv().is_err());
        Ok(())
    }
}
