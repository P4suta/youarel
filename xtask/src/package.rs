//! The archives a release ships, written byte for byte the same from the same inputs: gzip-compressed tar, zip, and Debian packages.
//!
//! Every entry has a fixed time, owner, and mode, so two builds of one candidate give archives with one digest.

use anyhow::{Context, Result, ensure};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use std::io::{Cursor, Read as _, Write as _};

/// The mode of an executable entry.
pub const EXECUTABLE: u32 = 0o755;
/// The mode of any other file entry.
pub const REGULAR: u32 = 0o644;
/// The permission bits of a mode.
const PERMISSIONS: u32 = 0o777;
/// The bytes of a kibibyte, the unit of a Debian package's installed size.
const KIBIBYTE: usize = 1024;

/// One file of an archive: its path inside, its mode, and its bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub path: String,
    pub mode: u32,
    pub bytes: Vec<u8>,
}

/// A gzip-compressed tar archive of `entries`, every one stamped with `time` and owned by root.
///
/// An entry whose path ends with `/` is a directory.
///
/// # Errors
///
/// Returns an error when an entry's path does not fit a tar header.
pub fn tar_gz(entries: &[Entry], time: u64) -> Result<Vec<u8>> {
    let compressor = GzEncoder::new(Vec::new(), Compression::best());
    let mut builder = tar::Builder::new(compressor);
    builder.mode(tar::HeaderMode::Deterministic);
    for entry in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(if entry.path.ends_with('/') {
            tar::EntryType::Directory
        } else {
            tar::EntryType::Regular
        });
        header.set_size(u64::try_from(entry.bytes.len())?);
        header.set_mode(entry.mode);
        header.set_mtime(time);
        header.set_uid(0);
        header.set_gid(0);
        builder
            .append_data(&mut header, &entry.path, entry.bytes.as_slice())
            .with_context(|| format!("add {} to the archive", entry.path))?;
    }
    let encoder = builder.into_inner().context("finish the tar archive")?;
    encoder.finish().context("finish the gzip stream")
}

/// The entries of a gzip-compressed tar archive, in order.
///
/// # Errors
///
/// Returns an error when the bytes are not such an archive.
pub fn read_tar_gz(bytes: &[u8]) -> Result<Vec<Entry>> {
    let mut archive = tar::Archive::new(GzDecoder::new(bytes));
    let mut found = Vec::new();
    for entry in archive.entries().context("read the tar archive")? {
        let mut entry = entry.context("read a tar entry")?;
        let path = entry
            .path()
            .context("read a tar path")?
            .display()
            .to_string();
        let mode = entry.header().mode().context("read a tar mode")?;
        let mut content = Vec::new();
        entry
            .read_to_end(&mut content)
            .context("read a tar entry's bytes")?;
        found.push(Entry {
            path,
            mode,
            bytes: content,
        });
    }
    Ok(found)
}

/// A zip archive of `entries`, deflated, with the earliest time a zip records and Unix modes on every host.
///
/// Left to itself, the writer marks an entry it makes on Windows as that host's, and the reader ignores the mode such an entry records.
///
/// # Errors
///
/// Returns an error when the writer fails.
pub fn zip(entries: &[Entry]) -> Result<Vec<u8>> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for entry in entries {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default())
            .system(zip::System::Unix)
            .unix_permissions(entry.mode);
        writer
            .start_file(entry.path.as_str(), options)
            .with_context(|| format!("add {} to the zip archive", entry.path))?;
        writer
            .write_all(&entry.bytes)
            .context("write a zip entry")?;
    }
    Ok(writer
        .finish()
        .context("finish the zip archive")?
        .into_inner())
}

/// The entries of a zip archive, in order.
///
/// # Errors
///
/// Returns an error when the bytes are not such an archive.
pub fn read_zip(bytes: &[u8]) -> Result<Vec<Entry>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context("read the zip archive")?;
    let mut found = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).context("read a zip entry")?;
        let mut content = Vec::new();
        file.read_to_end(&mut content)
            .context("read a zip entry's bytes")?;
        found.push(Entry {
            path: file.name().context("read a zip entry's name")?.into_owned(),
            mode: file.unix_mode().unwrap_or(REGULAR) & PERMISSIONS,
            bytes: content,
        });
    }
    Ok(found)
}

/// The global header of an `ar` archive.
const AR_MAGIC: &[u8] = b"!<arch>\n";
/// The bytes of one `ar` member header.
const AR_HEADER: usize = 60;
/// What ends an `ar` member header.
const AR_HEADER_END: &[u8] = b"`\n";
/// The boundary every `ar` member starts on, which a newline after an odd member keeps.
const AR_ALIGNMENT: usize = 2;
/// Where an `ar` member header holds the name.
const AR_NAME: core::ops::Range<usize> = 0..16;
/// Where an `ar` member header holds the size.
const AR_SIZE: core::ops::Range<usize> = 48..58;

/// One `ar` member header: the name, time, owner, group, mode, and size in their fixed-width fields.
fn ar_header(name: &str, size: usize, time: u64) -> Result<Vec<u8>> {
    let header = format!(
        "{name:<16}{time:<12}{:<6}{:<6}{:<8}{size:<10}",
        0, 0, "100644"
    );
    let mut bytes = header.into_bytes();
    bytes.extend_from_slice(AR_HEADER_END);
    ensure!(
        bytes.len() == AR_HEADER,
        "the ar member {name} does not fit its header"
    );
    Ok(bytes)
}

/// An `ar` archive of named members, which a Debian package is.
///
/// # Errors
///
/// Returns an error when a member's name or size does not fit its header.
pub fn ar(members: &[(&str, &[u8])], time: u64) -> Result<Vec<u8>> {
    let mut archive = AR_MAGIC.to_vec();
    for (name, bytes) in members {
        archive.extend(ar_header(name, bytes.len(), time)?);
        archive.extend_from_slice(bytes);
        if !bytes.len().is_multiple_of(AR_ALIGNMENT) {
            archive.push(b'\n');
        }
    }
    Ok(archive)
}

/// The members of an `ar` archive, by name.
///
/// # Errors
///
/// Returns an error when the bytes are not such an archive.
pub fn read_ar(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut rest = bytes
        .strip_prefix(AR_MAGIC)
        .context("the archive lacks the ar magic")?;
    let mut members = Vec::new();
    while !rest.is_empty() {
        let (header, after) = rest
            .split_at_checked(AR_HEADER)
            .context("an ar header is cut short")?;
        ensure!(header.ends_with(AR_HEADER_END), "an ar header is malformed");
        let field = |range: core::ops::Range<usize>| {
            header
                .get(range)
                .map(|field| String::from_utf8_lossy(field).trim().to_owned())
                .unwrap_or_default()
        };
        let name = field(AR_NAME);
        let size: usize = field(AR_SIZE)
            .parse()
            .context("an ar size is not a number")?;
        let (content, after) = after
            .split_at_checked(size)
            .context("an ar member is cut short")?;
        members.push((name, content.to_vec()));
        rest = if size.is_multiple_of(AR_ALIGNMENT) {
            after
        } else {
            after.get(1..).unwrap_or_default()
        };
    }
    Ok(members)
}

/// Every directory above the files, each ending with `/`, parents before their children.
fn directories(files: &[Entry]) -> Vec<String> {
    let mut found = std::collections::BTreeSet::new();
    for file in files {
        let mut prefix = String::new();
        let mut parts: Vec<&str> = file.path.split('/').collect();
        parts.pop();
        for part in parts {
            prefix.push_str(part);
            prefix.push('/');
            found.insert(prefix.clone());
        }
    }
    found.into_iter().collect()
}

/// What a Debian package declares about itself.
#[derive(Clone, Debug)]
pub struct Control<'a> {
    pub package: &'a str,
    pub version: &'a str,
    /// The Debian architecture, such as `amd64` or `arm64`.
    pub architecture: &'a str,
    pub maintainer: &'a str,
    pub homepage: &'a str,
    pub summary: &'a str,
}

/// A Debian package that installs `files`, which hold paths below the root such as `usr/bin/youarel`.
///
/// # Errors
///
/// Returns an error when an archive cannot be written.
pub fn deb(control: &Control<'_>, files: &[Entry], time: u64) -> Result<Vec<u8>> {
    let installed: usize = files.iter().map(|file| file.bytes.len()).sum();
    let kibibytes = installed.div_ceil(KIBIBYTE);
    let text = format!(
        "Package: {}\nVersion: {}\nArchitecture: {}\nMaintainer: {}\nInstalled-Size: {kibibytes}\nSection: web\nPriority: optional\nHomepage: {}\nDescription: {}\n",
        control.package,
        control.version,
        control.architecture,
        control.maintainer,
        control.homepage,
        control.summary
    );
    let control_entry = Entry {
        path: String::from("control"),
        mode: REGULAR,
        bytes: text.into_bytes(),
    };
    let control_archive = tar_gz(&[control_entry], time)?;
    let mut tree: Vec<Entry> = directories(files)
        .into_iter()
        .map(|path| Entry {
            path,
            mode: EXECUTABLE,
            bytes: Vec::new(),
        })
        .collect();
    tree.extend_from_slice(files);
    let data_archive = tar_gz(&tree, time)?;
    ar(
        &[
            ("debian-binary", b"2.0\n"),
            ("control.tar.gz", &control_archive),
            ("data.tar.gz", &data_archive),
        ],
        time,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        Control, EXECUTABLE, Entry, REGULAR, ar, deb, read_ar, read_tar_gz, read_zip, tar_gz, zip,
    };

    fn entries() -> Vec<Entry> {
        vec![
            Entry {
                path: String::from("youarel-1.0.0/youarel"),
                mode: EXECUTABLE,
                bytes: b"\x7fELF program".to_vec(),
            },
            Entry {
                path: String::from("youarel-1.0.0/README.md"),
                mode: REGULAR,
                bytes: b"# youarel\n".to_vec(),
            },
        ]
    }

    #[test]
    fn archives_read_back_and_repeat_byte_for_byte() {
        let written = tar_gz(&entries(), 1_700_000_000).unwrap();
        assert_eq!(read_tar_gz(&written).unwrap(), entries());
        assert_eq!(tar_gz(&entries(), 1_700_000_000).unwrap(), written);
        let zipped = zip(&entries()).unwrap();
        assert_eq!(read_zip(&zipped).unwrap(), entries());
        assert_eq!(zip(&entries()).unwrap(), zipped);
    }

    #[test]
    fn a_debian_package_holds_its_three_members_in_order() {
        let control = Control {
            package: "youarel",
            version: "1.0.0",
            architecture: "amd64",
            maintainer: "P4suta",
            homepage: "https://github.com/P4suta/youarel",
            summary: "web shortcuts",
        };
        let file = Entry {
            path: String::from("usr/bin/youarel"),
            mode: EXECUTABLE,
            bytes: vec![1; 3000],
        };
        let package = deb(&control, &[file], 0).unwrap();
        let members = read_ar(&package).unwrap();
        let names: Vec<&str> = members.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["debian-binary", "control.tar.gz", "data.tar.gz"]);
        assert_eq!(members[0].1, b"2.0\n");
        let control_text =
            String::from_utf8(read_tar_gz(&members[1].1).unwrap()[0].bytes.clone()).unwrap();
        assert!(
            control_text.contains("Installed-Size: 3\n"),
            "{control_text}"
        );
        let data = read_tar_gz(&members[2].1).unwrap();
        let paths: Vec<&str> = data.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(paths, ["usr/", "usr/bin/", "usr/bin/youarel"]);
        assert_eq!(data[2].mode, EXECUTABLE);
        assert_eq!(data[0].mode, EXECUTABLE);
    }

    #[test]
    fn odd_members_are_padded_and_read_back() {
        let archive = ar(&[("a", b"odd"), ("b", b"even")], 5).unwrap();
        assert_eq!(archive.len(), 8 + 60 + 4 + 60 + 4);
        let members = read_ar(&archive).unwrap();
        assert_eq!(
            members,
            [
                (String::from("a"), b"odd".to_vec()),
                (String::from("b"), b"even".to_vec())
            ]
        );
        read_ar(b"not an archive").unwrap_err();
    }
}
