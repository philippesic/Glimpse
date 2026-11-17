use std::{ffi::OsString, path::PathBuf};
pub fn informational(args: &[OsString]) -> Option<&'static str> {
    let flags: Vec<_> = args.iter().take_while(|arg| *arg != "--").collect();
    if flags.iter().any(|arg| *arg == "--help" || *arg == "-h") {
        Some("help")
    } else if flags.iter().any(|arg| *arg == "--version") {
        Some("version")
    } else {
        None
    }
}
#[derive(Default, Debug)]
pub struct Request {
    pub desktop: bool,
    pub files: Vec<PathBuf>,
    pub folder: Option<PathBuf>,
    pub scope: Option<PathBuf>,
    pub wait: bool,
    pub line: usize,
    pub column: usize,
    pub pick: bool,
    pub directory: bool,
    pub nul: bool,
}
pub fn parse(args: Vec<OsString>, cwd: PathBuf) -> Result<Request, String> {
    let mut request = Request {
        desktop: args.is_empty(),
        line: 1,
        column: 1,
        ..Default::default()
    };
    let mut args = args.into_iter().peekable();
    let command = args.peek().and_then(|s| s.to_str()).unwrap_or("");
    let open = command == "open";
    if open || command == "pick" {
        request.pick = command == "pick";
        args.next();
    }
    let mut literal = false;
    while let Some(arg) = args.next() {
        if !literal {
            match arg.to_str() {
                Some("--") => {
                    literal = true;
                    continue;
                }
                Some("--wait") if !request.pick => {
                    request.wait = true;
                    continue;
                }
                Some("--directory") if request.pick => {
                    request.directory = true;
                    continue;
                }
                Some("--scope") => {
                    let path = args.next().ok_or("--scope requires a folder")?;
                    let path = cwd.join(path);
                    if !path.is_dir() {
                        return Err("Scope must be an existing folder".into());
                    }
                    request.scope = Some(path);
                    continue;
                }
                Some("--line" | "--column") if !request.pick => {
                    let value: usize = args
                        .next()
                        .and_then(|s| s.to_str().and_then(|s| s.parse().ok()))
                        .ok_or("Position requires a positive integer")?;
                    if value == 0 {
                        return Err("Positions begin at one".into());
                    }
                    if arg == "--line" {
                        request.line = value;
                    } else {
                        request.column = value;
                    }
                    continue;
                }
                Some("--format") if request.pick => {
                    let format = args.next().ok_or("Missing format")?;
                    request.nul = format == "nul";
                    if !request.nul && format != "json" {
                        return Err("Format must be json or nul".into());
                    }
                    continue;
                }
                Some(s) if s.starts_with('-') => return Err(format!("Unknown option {s}")),
                _ => {}
            }
        }
        let path = cwd.join(arg);
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.is_dir() {
            if open || request.folder.is_some() {
                return Err("At most one initial folder; open accepts files only".into());
            }
            request.folder = Some(path);
        } else if meta.is_file() {
            request.files.push(path);
        } else {
            return Err("Only files and folders are supported".into());
        }
    }
    if request.pick && !request.files.is_empty() {
        return Err("Picker start must be a directory".into());
    }
    if open && request.files.is_empty() {
        return Err("open requires at least one file".into());
    }
    if !request.desktop && request.folder.is_none() && request.files.is_empty() {
        request.folder = Some(cwd);
    }
    if request.wait && request.files.is_empty() {
        return Err("--wait requires at least one file".into());
    }
    if request.wait {
        for path in &request.files {
            crate::document::DiskFile::open(path.clone())?;
        }
    }
    Ok(request)
}
pub fn selection_json(path: &std::path::Path, directory: bool) -> serde_json::Value {
    use base64::Engine;
    use std::os::unix::ffi::OsStrExt;
    let mut value = serde_json::json!({"version":1,"kind":if directory{"directory"}else{"file"},"path":{"encoding":"unix-bytes","base64":base64::engine::general_purpose::STANDARD.encode(path.as_os_str().as_bytes())}});
    if let Some(text) = path.to_str() {
        value["path"]["utf8"] = text.into();
    }
    value
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_a_plain_desktop_launch_restores_abandoned_work() {
        let root = tempfile::tempdir().unwrap();
        let desktop = parse(vec![], root.path().into()).unwrap();
        assert!(desktop.desktop);
        assert!(desktop.folder.is_none());
        let explicit = parse(vec![".".into()], root.path().into()).unwrap();
        assert!(!explicit.desktop);
        assert_eq!(explicit.folder, Some(root.path().join(".")));
        assert!(
            !parse(vec!["pick".into()], root.path().into())
                .unwrap()
                .desktop
        );
        assert!(parse(vec!["open".into()], root.path().into()).is_err());
        assert!(parse(vec!["--wait".into()], root.path().into()).is_err());
    }
    #[test]
    fn literal_option_names_are_paths_not_information_requests() {
        let root = tempfile::tempdir().unwrap();
        for name in ["--help", "-h", "--version", "open", "pick"] {
            std::fs::write(root.path().join(name), "text").unwrap();
            let args = vec!["--".into(), name.into()];
            assert_eq!(informational(&args), None);
            assert_eq!(
                parse(args, root.path().into()).unwrap().files,
                [root.path().join(name)]
            );
        }
        assert_eq!(
            informational(&["open".into(), "--help".into()]),
            Some("help")
        );
    }
    #[test]
    fn picker_output_preserves_native_bytes_and_omits_lossy_utf8() {
        use base64::Engine;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/a\xff\n".to_vec()));
        let value = selection_json(&path, false);
        assert!(value["path"].get("utf8").is_none());
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(value["path"]["base64"].as_str().unwrap())
                .unwrap(),
            path.as_os_str().as_bytes()
        );
    }
    #[test]
    fn wait_after_paths_still_preflights_every_target() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("binary"), [0, 255]).unwrap();
        assert!(
            parse(
                vec!["open".into(), "binary".into(), "--wait".into()],
                root.path().into()
            )
            .is_err()
        );
    }
    #[test]
    fn arguments_are_validated_before_ui() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a b"), "ok").unwrap();
        let r = parse(
            vec![
                "open".into(),
                "--wait".into(),
                "--line".into(),
                "3".into(),
                "--".into(),
                "a b".into(),
            ],
            dir.path().to_owned(),
        )
        .unwrap();
        assert!(r.wait);
        assert_eq!(r.line, 3);
        assert_eq!(r.files[0], dir.path().join("a b"));
        assert!(parse(vec!["absent".into()], dir.path().to_owned()).is_err());
        assert!(parse(vec!["open".into(), ".".into()], dir.path().to_owned()).is_err());
        assert!(parse(vec![".".into(), ".".into()], dir.path().to_owned()).is_err());
    }
}
