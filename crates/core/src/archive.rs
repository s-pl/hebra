//! Archives of a tree: tar, tar.gz and zip, like `git archive`. Written by
//! hand because the formats are small and a dependency would be larger than
//! this file. Timestamps are fixed by the caller, so archives are reproducible.

use std::io::Write;

use crate::attributes::Rules;
use crate::error::{Error, Result};
use crate::model::Tree;
use crate::store::{BLOB, Store};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Tar,
    TarGz,
    Zip,
}

impl Format {
    /// From a name (`tar`, `tgz`, `tar.gz`, `zip`).
    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "tar" => Ok(Self::Tar),
            "tgz" | "tar.gz" => Ok(Self::TarGz),
            "zip" => Ok(Self::Zip),
            other => Err(Error::InvalidArgument(format!(
                "unknown archive format `{other}`"
            ))),
        }
    }

    /// From a file name, tar if nothing matches.
    #[must_use]
    pub fn for_file(name: &str) -> Self {
        if name.ends_with(".zip") {
            Self::Zip
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Self::TarGz
        } else {
            Self::Tar
        }
    }
}

struct Member {
    path: String,
    data: Vec<u8>,
    executable: bool,
    link: Option<String>,
}

fn members(store: &Store, tree: &Tree, prefix: &str) -> Result<Vec<Member>> {
    let rules = match tree.files.get(".gitattributes") {
        Some(entry) => Rules::parse(&String::from_utf8_lossy(&store.read(BLOB, &entry.blob)?)),
        None => Rules::default(),
    };
    let mut out = Vec::with_capacity(tree.files.len());
    for (path, entry) in &tree.files {
        if entry.gitlink || rules.of(path).export_ignore {
            continue;
        }
        let data = store.read(BLOB, &entry.blob)?;
        let path = format!("{prefix}{path}");
        if entry.symlink {
            out.push(Member {
                path,
                data: Vec::new(),
                executable: false,
                link: Some(String::from_utf8_lossy(&data).into_owned()),
            });
        } else {
            out.push(Member {
                path,
                data,
                executable: entry.executable,
                link: None,
            });
        }
    }
    Ok(out)
}

/// A `.tar.gz` of files given as `(path, content, executable)`: build
/// artifacts, package tarballs.
pub fn tar_gz(files: &[(String, Vec<u8>, bool)], mtime: u64) -> Result<Vec<u8>> {
    let members: Vec<Member> = files
        .iter()
        .map(|(path, data, executable)| Member {
            path: path.clone(),
            data: data.clone(),
            executable: *executable,
            link: None,
        })
        .collect();
    Ok(gzip(&tar(&members, mtime)?, mtime))
}

/// Writes `tree` in the given format; `mtime` is Unix seconds.
pub fn write(
    store: &Store,
    tree: &Tree,
    format: Format,
    prefix: &str,
    mtime: u64,
    out: &mut impl Write,
) -> Result<()> {
    let files = members(store, tree, prefix)?;
    let bytes = match format {
        Format::Tar => tar(&files, mtime)?,
        Format::TarGz => gzip(&tar(&files, mtime)?, mtime),
        Format::Zip => zip(&files, mtime),
    };
    out.write_all(&bytes).map_err(|source| Error::Io {
        path: "archive".into(),
        source,
    })
}

fn octal(field: &mut [u8], value: u64) {
    let width = field.len() - 1;
    let text = format!("{value:0width$o}");
    let bytes = text.as_bytes();
    let start = bytes.len().saturating_sub(width);
    field[..width].copy_from_slice(&bytes[start..]);
}

/// A tar header, with the long-name extension when the path does not fit.
fn tar_header(path: &str, size: u64, mode: u64, kind: u8, link: &str, mtime: u64) -> [u8; 512] {
    let mut h = [0u8; 512];
    let name = path.as_bytes();
    let take = name.len().min(100);
    h[..take].copy_from_slice(&name[..take]);
    octal(&mut h[100..108], mode);
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size);
    octal(&mut h[136..148], mtime);
    h[148..156].copy_from_slice(b"        ");
    h[156] = kind;
    let target = link.as_bytes();
    let take = target.len().min(100);
    h[157..157 + take].copy_from_slice(&target[..take]);
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    let sum: u64 = h.iter().map(|b| u64::from(*b)).sum();
    let text = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(text.as_bytes());
    h
}

fn pad(out: &mut Vec<u8>) {
    let rest = out.len() % 512;
    if rest != 0 {
        out.resize(out.len() + 512 - rest, 0);
    }
}

fn tar(files: &[Member], mtime: u64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for file in files {
        if file.path.len() > 100 {
            // GNU long name: a pseudo file holding the name.
            let name = file.path.as_bytes();
            let header = tar_header(
                "././@LongLink",
                name.len() as u64 + 1,
                0o644,
                b'L',
                "",
                mtime,
            );
            out.extend_from_slice(&header);
            out.extend_from_slice(name);
            out.push(0);
            pad(&mut out);
        }
        let (mode, kind, link) = match &file.link {
            Some(target) => (0o777, b'2', target.as_str()),
            None if file.executable => (0o755, b'0', ""),
            None => (0o644, b'0', ""),
        };
        out.extend_from_slice(&tar_header(
            &file.path,
            file.data.len() as u64,
            mode,
            kind,
            link,
            mtime,
        ));
        out.extend_from_slice(&file.data);
        pad(&mut out);
    }
    out.resize(out.len() + 1024, 0);
    Ok(out)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn gzip(data: &[u8], mtime: u64) -> Vec<u8> {
    let body = miniz_oxide::deflate::compress_to_vec(data, 6);
    let mut out = Vec::with_capacity(body.len() + 18);
    out.extend_from_slice(&[0x1f, 0x8b, 8, 0]);
    out.extend_from_slice(&u32::try_from(mtime).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&[0, 0xff]);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(data.len() % (1 << 32))
            .unwrap_or(0)
            .to_le_bytes(),
    );
    out
}

/// MS-DOS date and time, as zip wants them.
fn dos_time(mtime: u64) -> (u16, u16) {
    let Ok(ts) = jiff::Timestamp::from_second(i64::try_from(mtime).unwrap_or(0)) else {
        return (0, 0x21);
    };
    let d = ts.to_zoned(jiff::tz::TimeZone::UTC);
    let time =
        (i32::from(d.hour()) << 11) | (i32::from(d.minute()) << 5) | (i32::from(d.second()) / 2);
    let year = (i32::from(d.year()) - 1980).clamp(0, 127);
    let date = (year << 9) | (i32::from(d.month()) << 5) | i32::from(d.day());
    (
        u16::try_from(time).unwrap_or(0),
        u16::try_from(date).unwrap_or(0x21),
    )
}

fn zip(files: &[Member], mtime: u64) -> Vec<u8> {
    let (time, date) = dos_time(mtime);
    let mut out = Vec::new();
    let mut central = Vec::new();
    for file in files {
        let content: &[u8] = match &file.link {
            Some(target) => target.as_bytes(),
            None => &file.data,
        };
        let crc = crc32(content);
        let packed = miniz_oxide::deflate::compress_to_vec(content, 6);
        let (method, body): (u16, &[u8]) = if packed.len() < content.len() {
            (8, &packed)
        } else {
            (0, content)
        };
        let offset = u32::try_from(out.len()).unwrap_or(u32::MAX);
        let name = file.path.as_bytes();
        let sizes = [
            u32::try_from(body.len()).unwrap_or(u32::MAX),
            u32::try_from(content.len()).unwrap_or(u32::MAX),
        ];
        let header = |sig: &[u8], central_entry: bool, out: &mut Vec<u8>| {
            out.extend_from_slice(sig);
            if central_entry {
                out.extend_from_slice(&0x0314u16.to_le_bytes());
            }
            out.extend_from_slice(&20u16.to_le_bytes());
            // UTF-8 names.
            out.extend_from_slice(&0x0800u16.to_le_bytes());
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&time.to_le_bytes());
            out.extend_from_slice(&date.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&sizes[0].to_le_bytes());
            out.extend_from_slice(&sizes[1].to_le_bytes());
            out.extend_from_slice(&u16::try_from(name.len()).unwrap_or(u16::MAX).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
        };
        header(b"PK\x03\x04", false, &mut out);
        out.extend_from_slice(name);
        out.extend_from_slice(body);

        header(b"PK\x01\x02", true, &mut central);
        central.extend_from_slice(&[0; 6]);
        let mode: u32 = match (&file.link, file.executable) {
            (Some(_), _) => 0o120_777,
            (None, true) => 0o100_755,
            (None, false) => 0o100_644,
        };
        central.extend_from_slice(&(mode << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let start = u32::try_from(out.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&[0; 4]);
    let count = u16::try_from(files.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(central.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn tar_headers_have_valid_checksums() {
        let h = tar_header("a.txt", 5, 0o644, b'0', "", 0);
        let mut copy = h;
        copy[148..156].copy_from_slice(b"        ");
        let sum: u64 = copy.iter().map(|b| u64::from(*b)).sum();
        let stored = std::str::from_utf8(&h[148..154]).unwrap();
        assert_eq!(u64::from_str_radix(stored, 8).unwrap(), sum);
    }

    #[test]
    fn gzip_round_trips() {
        let data = b"hello hello hello hello".repeat(50);
        let packed = gzip(&data, 1);
        assert_eq!(&packed[..2], &[0x1f, 0x8b]);
        let inner = &packed[10..packed.len() - 8];
        assert_eq!(
            miniz_oxide::inflate::decompress_to_vec(inner).unwrap(),
            data
        );
    }
}
