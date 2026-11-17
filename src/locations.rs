use std::path::PathBuf;

#[derive(Clone, Default)]
pub struct Volumes {
    pub paths: Vec<PathBuf>,
    pub note: Option<String>,
}

pub fn mounted() -> Result<Volumes, String> {
    platform::mounted()
}
pub fn mount_points() -> Result<Vec<PathBuf>, String> {
    platform::mount_points()
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::{io::Read, os::unix::ffi::OsStringExt};

    fn unescape(bytes: &[u8]) -> Option<Vec<u8>> {
        let mut result = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'\\' {
                let digits = bytes.get(index + 1..index + 4)?;
                if !digits.iter().all(|byte| (b'0'..=b'7').contains(byte)) {
                    return None;
                }
                let value = digits
                    .iter()
                    .fold(0u16, |value, byte| value * 8 + (byte - b'0') as u16);
                result.push(u8::try_from(value).ok()?);
                index += 4;
            } else {
                result.push(bytes[index]);
                index += 1;
            }
        }
        (!result.contains(&0)).then_some(result)
    }

    fn paths(bytes: &[u8]) -> Vec<PathBuf> {
        let mut paths = vec![PathBuf::from("/")];
        for line in bytes.split(|byte| *byte == b'\n') {
            let fields: Vec<_> = line
                .split(|byte| byte.is_ascii_whitespace())
                .filter(|field| !field.is_empty())
                .collect();
            let Some(separator) = fields.iter().position(|field| *field == b"-") else {
                continue;
            };
            if separator < 6 || fields.len() <= separator + 3 {
                continue;
            }
            let filesystem = fields[separator + 1];
            let Some(path) = unescape(fields[4]) else {
                continue;
            };
            let path = PathBuf::from(std::ffi::OsString::from_vec(path));
            if !path.is_absolute() {
                continue;
            }
            let user_mount = path.starts_with("/mnt")
                || path.starts_with("/media")
                || path.starts_with("/run/media");
            if matches!(
                filesystem,
                b"proc"
                    | b"sysfs"
                    | b"devtmpfs"
                    | b"devpts"
                    | b"cgroup"
                    | b"cgroup2"
                    | b"securityfs"
                    | b"debugfs"
                    | b"tracefs"
                    | b"pstore"
                    | b"configfs"
                    | b"efivarfs"
                    | b"mqueue"
                    | b"hugetlbfs"
                    | b"fusectl"
                    | b"autofs"
                    | b"binfmt_misc"
                    | b"rpc_pipefs"
                    | b"nsfs"
                    | b"selinuxfs"
                    | b"bpf"
                    | b"ramfs"
            ) || (filesystem == b"tmpfs" && !user_mount)
            {
                continue;
            }
            paths.push(path);
        }
        paths.sort();
        paths.dedup();
        paths
    }

    pub fn mount_points() -> Result<Vec<PathBuf>, String> {
        let mut bytes = Vec::new();
        std::fs::File::open("/proc/self/mountinfo")
            .and_then(|file| file.take(1024 * 1024 + 1).read_to_end(&mut bytes))
            .map_err(|error| error.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("Mount table exceeds the search safety budget".into());
        }
        let mut result = Vec::new();
        for line in bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            let path = line
                .split(|byte| byte.is_ascii_whitespace())
                .filter(|field| !field.is_empty())
                .nth(4)
                .and_then(unescape)
                .ok_or("Invalid mount table; search was not started")?;
            let path = PathBuf::from(std::ffi::OsString::from_vec(path));
            if !path.is_absolute() {
                return Err("Invalid relative mount boundary".into());
            }
            result.push(path);
        }
        Ok(result)
    }

    pub fn mounted() -> Result<Volumes, String> {
        let mut bytes = Vec::new();
        #[cfg(feature = "test-support")]
        let source = std::env::var_os("GLIMPSE_TEST_MOUNTS_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/proc/self/mountinfo"));
        #[cfg(not(feature = "test-support"))]
        let source = PathBuf::from("/proc/self/mountinfo");
        std::fs::File::open(source)
            .and_then(|file| file.take(1024 * 1024 + 1).read_to_end(&mut bytes))
            .map_err(|error| error.to_string())?;
        let truncated = bytes.len() > 1024 * 1024;
        if truncated {
            bytes.truncate(1024 * 1024);
            if let Some(last) = bytes.iter().rposition(|byte| *byte == b'\n') {
                bytes.truncate(last + 1);
            }
        }
        let mut paths = paths(&bytes);
        let limited = paths.len() > 128;
        paths.truncate(128);
        Ok(Volumes {
            paths,
            note: (limited || truncated).then(|| {
                "Mounted-location list is incomplete; use Location to open another path.".into()
            }),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn mount_points_preserve_native_bytes_and_ignore_optional_fields() {
            let input = b"36 35 98:0 / /media/a\\040b rw shared:7 future:tag - ext4 /dev/root rw\n37 35 98:1 / /mnt/c\\134d\\377 rw - nfs server:path rw\n38 35 0:1 / /proc rw - proc proc rw\n39 35 0:2 / /run rw - tmpfs none rw\n40 35 0:3 / /mnt/ram rw - tmpfs none rw\n";
            let actual = paths(input);
            assert_eq!(
                actual,
                vec![
                    PathBuf::from("/"),
                    PathBuf::from("/media/a b"),
                    PathBuf::from(std::ffi::OsString::from_vec(b"/mnt/c\\d\xff".to_vec())),
                    PathBuf::from("/mnt/ram")
                ]
            );
            assert_eq!(unescape(b"a\\012b\\011c"), Some(b"a\nb\tc".to_vec()));
            assert!(unescape(b"a\\999").is_none());
            assert!(unescape(b"a\\000").is_none());
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    pub fn mount_points() -> Result<Vec<PathBuf>, String> {
        use std::os::unix::ffi::OsStringExt;
        let count = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
        if count <= 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let capacity = count as usize + 16;
        let bytes = capacity
            .checked_mul(std::mem::size_of::<libc::statfs>())
            .filter(|size| *size <= 16 * 1024 * 1024)
            .ok_or("Mount table exceeds the search safety budget")?;
        let mut entries: Vec<std::mem::MaybeUninit<libc::statfs>> =
            std::iter::repeat_with(std::mem::MaybeUninit::uninit)
                .take(capacity)
                .collect();
        let written = unsafe {
            libc::getfsstat(
                entries.as_mut_ptr().cast(),
                bytes as libc::c_int,
                libc::MNT_NOWAIT,
            )
        };
        if written < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if written as usize >= capacity {
            return Err("Mount table changed during discovery; retry search".into());
        }
        Ok(entries
            .iter()
            .take(written as usize)
            .map(|entry| {
                let entry = unsafe { entry.assume_init_ref() };
                PathBuf::from(std::ffi::OsString::from_vec(
                    entry
                        .f_mntonname
                        .iter()
                        .take_while(|&&byte| byte != 0)
                        .map(|&byte| byte as u8)
                        .collect(),
                ))
            })
            .collect())
    }

    pub fn mounted() -> Result<Volumes, String> {
        let mut paths: Vec<_> = mount_points()?
            .into_iter()
            .filter(|path| {
                path.as_path() == std::path::Path::new("/") || path.starts_with("/Volumes")
            })
            .collect();
        paths.sort();
        paths.dedup();
        let truncated = paths.len() > 128;
        paths.truncate(128);
        Ok(Volumes {
            paths,
            note: truncated.then(|| {
                "Mounted-location list is incomplete; use Location to open another path.".into()
            }),
        })
    }
}
