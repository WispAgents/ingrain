//! JSON Lines files for captured data and the hit log.

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

/// Read every record. A missing file is an empty list.
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    BufReader::new(file)
        .lines()
        .enumerate()
        .filter(|(_, line)| line.as_ref().map_or(true, |l| !l.trim().is_empty()))
        .map(|(i, line)| {
            let line = line?;
            serde_json::from_str(&line)
                .with_context(|| format!("{}:{}: invalid record", path.display(), i + 1))
        })
        .collect()
}

/// Replace the file with these records.
pub fn write<T: Serialize>(path: &Path, records: &[T]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("jsonl.tmp");
    {
        let mut out = BufWriter::new(File::create(&tmp)?);
        for record in records {
            serde_json::to_writer(&mut out, record)?;
            out.write_all(b"\n")?;
        }
        out.flush()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
}

/// Append records to the end of the file.
pub fn append<T: Serialize>(path: &Path, records: &[T]) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut out = BufWriter::new(file);
    for record in records {
        serde_json::to_writer(&mut out, record)?;
        out.write_all(b"\n")?;
    }
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_append_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.jsonl");
        assert!(read::<u32>(&path).unwrap().is_empty());
        write(&path, &[1u32, 2]).unwrap();
        append(&path, &[3u32]).unwrap();
        assert_eq!(read::<u32>(&path).unwrap(), vec![1, 2, 3]);
    }
}
