//! A.15 admission of the include closure reported by root's staged nginx -T.
use crate::{db_upgrade::key_file::{validate_trusted_input, KeyFileError, TrustedInput}, upgrade::StepError};
use std::{ffi::OsStr, io::{BufRead, Read}, os::unix::ffi::OsStrExt, path::Path};

pub(crate) const MAX_NGINX_T_OUTPUT: usize = 256 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const HEADER_PREFIX: &[u8] = b"# configuration file ";
const MAX_HEADER_BYTES: usize = HEADER_PREFIX.len() + MAX_PATH_BYTES + 1;

fn malformed(reason: &str) -> StepError {
    let mut error = KeyFileError::Unsafe.trusted_input_failure(Path::new("<nginx -T>"));
    error.detail = Some(format!("nginx -T output: {reason}")); error
}
fn admit_header(line: &[u8], overflow: bool, last: Option<u8>) -> Result<bool, StepError> {
    if !line.starts_with(HEADER_PREFIX) || line.get(HEADER_PREFIX.len()) != Some(&b'/') || last != Some(b':') {
        return Ok(false);
    }
    if overflow { return Err(malformed("configuration-file path limit exceeded")); }
    let name = &line[HEADER_PREFIX.len()..line.len() - 1];
    if name.contains(&0) { return Err(malformed("NUL in configuration-file path")); }
    let path = Path::new(OsStr::from_bytes(name));
    let result=match super::store::PublisherStore::trusted_directory(){
        Some(root)=>crate::db_upgrade::key_file::validate_publisher_input(path,&root),
        None=>validate_trusted_input(path,TrustedInput::Config),
    };
    result.map_err(|cause| {
        let mut error = cause.trusted_input_failure(path);
        if cause == KeyFileError::Missing { error.detail = Some(format!("file not found: {}", path.display())); }
        error
    })?;
    Ok(true)
}

#[cfg(test)]
pub(crate) fn validate(output: &[u8]) -> Result<(), StepError> { validate_stream(output) }

pub(crate) fn validate_stream(reader: impl BufRead) -> Result<(), StepError> {
    if !validate_dump(reader)? { return Err(malformed("no configuration-file headers")); }
    Ok(())
}

// Empty stdout is only acceptable when the independently observed child exit rejects the config.
pub(super) fn validate_dump(mut reader: impl BufRead) -> Result<bool, StepError> {
    // Only header candidates accumulate; ordinary content is counted and skipped as bytes.
    let mut line = Vec::with_capacity(MAX_HEADER_BYTES);
    let (mut candidate, mut overflow, mut last) = (true, false, None);
    let (mut total, mut headers) = (0, 0);
    loop {
        if !candidate {
            let remaining = MAX_NGINX_T_OUTPUT - total;
            let skipped = Read::by_ref(&mut reader).take((remaining + 1) as u64).skip_until(b'\n')
                .map_err(|_| malformed("failed to read output"))?;
            if skipped > remaining { return Err(malformed("output limit exceeded")); }
            total += skipped; candidate = true; last = None;
            continue;
        }
        let (consumed, newline) = {
            let available = reader.fill_buf().map_err(|_| malformed("failed to read output"))?;
            if available.is_empty() { break; }
            let available = &available[..available.len().min(MAX_NGINX_T_OUTPUT - total + 1)];
            let consumed = available.iter().position(|&b| b == b'\n').map_or(available.len(), |i| i + 1);
            if consumed > MAX_NGINX_T_OUTPUT - total { return Err(malformed("output limit exceeded")); }
            let newline = available[consumed - 1] == b'\n';
            let content = &available[..consumed - usize::from(newline)];
            if candidate && !overflow {
                for &byte in content {
                    let position = line.len();
                    if (position < HEADER_PREFIX.len() && byte != HEADER_PREFIX[position])
                        || (position == HEADER_PREFIX.len() && byte != b'/') {
                        candidate = false; line.clear(); break;
                    }
                    if position == MAX_HEADER_BYTES { overflow = true; break; }
                    line.push(byte);
                }
            }
            if let Some(&byte) = content.last() { last = Some(byte); }
            (consumed, newline)
        };
        reader.consume(consumed); total += consumed;
        if newline {
            headers += usize::from(admit_header(&line, overflow, last)?);
            line.clear(); candidate = true; overflow = false; last = None;
        }
    }
    headers += usize::from(admit_header(&line, overflow, last)?);
    if total == 0 { return Ok(false); }
    if headers == 0 { return Err(malformed("no configuration-file headers")); }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::CString, fs::{self, File}, os::{fd::AsRawFd, unix::fs::{MetadataExt, PermissionsExt, symlink}}, path::{Path, PathBuf}};

    fn fixture() -> tempfile::TempDir {
        assert_eq!(unsafe { libc::geteuid() }, 0, "trusted-input tests require the root CI container");
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap(); dir
    }
    fn config(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name); fs::write(&path, "# root-owned fixture\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap(); path
    }
    fn dump(paths: &[&Path]) -> Vec<u8> {
        paths.iter().map(|p| format!("# configuration file {}:\n# root-owned fixture\n\n", p.display())).collect::<String>().into_bytes()
    }
    fn unsafe_error(bytes: &[u8]) -> StepError {
        let error = validate(bytes).unwrap_err(); assert_eq!(error.code, "trusted_input_unsafe");
        for guidance in ["chown root:root <path>", "chmod go-w <path>", "UID/GID 10001", "nginx/adm", "ACL"] {
            assert!(error.message.contains(guidance));
        }
        error
    }
    #[test]
    fn admits_real_dump_shape_with_expanded_globs_spaces_and_literal_quotes() {
        let dir = fixture(); let main = config(dir.path(), "nginx.conf");
        let first = config(dir.path(), "a space.conf"); let second = config(dir.path(), "b literal \"quotes\": value.conf");
        // nginx prints raw absolute filenames, even when include arguments were quoted.
        let output = format!("nginx: the configuration file {} syntax is ok\nnginx: configuration file {} test is successful\n# configuration file {}:\nevents {{}}\nhttp {{ include \"{}/*.conf\"; }}\n\n# configuration file {}:\nmap $host $fixture_a {{ default a; }}\n\n# configuration file {}:\nmap $host $fixture_b {{ default b; }}\n",
            main.display(), main.display(), main.display(), dir.path().display(), first.display(), second.display());
        validate(output.as_bytes()).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"# root-owned fixture\n");
    }
    #[test]
    fn every_header_is_admitted_and_api_owned_includes_are_preserved() {
        for index in 0..3 {
            let dir = fixture(); let paths = [config(dir.path(), "one.conf"), config(dir.path(), "two.conf"), config(dir.path(), "three.conf")];
            let path = &paths[index]; let raw = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            assert_eq!(unsafe { libc::chown(raw.as_ptr(), 10001, 10001) }, 0);
            let before = fs::metadata(path).unwrap(); let bytes = fs::read(path).unwrap();
            let refs = paths.iter().map(PathBuf::as_path).collect::<Vec<_>>();
            let error = unsafe_error(&dump(&refs)); assert_eq!(error.detail.as_deref(), path.to_str());
            let after = fs::metadata(path).unwrap();
            assert_eq!((after.ino(), after.mode(), after.uid(), after.gid()), (before.ino(), before.mode(), before.uid(), before.gid()));
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
    #[test]
    fn acl_writable_include_is_refused_by_the_shared_kernel_probe() {
        let dir = fixture(); let path = config(dir.path(), "acl.conf"); let file = File::open(&path).unwrap();
        let mut acl = 2u32.to_le_bytes().to_vec();
        for (tag, permission, id) in [(1u16, 6u16, u32::MAX), (2, 2, 10001), (4, 0, u32::MAX), (16, 2, u32::MAX), (32, 0, u32::MAX)] {
            acl.extend(tag.to_le_bytes()); acl.extend(permission.to_le_bytes()); acl.extend(id.to_le_bytes());
        }
        assert_eq!(unsafe { libc::fsetxattr(file.as_raw_fd(), c"system.posix_acl_access".as_ptr(), acl.as_ptr().cast(), acl.len(), 0) }, 0);
        let before = fs::metadata(&path).unwrap();
        let error = unsafe_error(&dump(&[&path])); assert_eq!(error.detail.as_deref(), path.to_str());
        assert_eq!(fs::metadata(&path).unwrap().mode(), before.mode());
        assert_eq!(fs::read(&path).unwrap(), b"# root-owned fixture\n");
    }
    #[test]
    fn fake_and_indented_header_comments_are_ignored_after_a_real_header() {
        let dir = fixture(); let path = config(dir.path(), "nginx.conf"); let good = String::from_utf8(dump(&[&path])).unwrap();
        for bad in ["# configuration file", "# configuration file ", "# configuration file :", "# configuration file /fixture/missing-colon",
            "# configuration file relative.conf:", "# configuration file for site X:", "# configuration file \"/fixture/quoted-header.conf\":", "# configuration file /fixture/path: trailing",
            " # configuration file /fixture/indented:"] {
            validate(format!("{good}{bad}\n").as_bytes()).unwrap();
        }
    }
    #[test]
    fn missing_files_have_the_shared_guidance_and_file_not_found_detail() {
        let dir = fixture();
        for path in [dir.path().join("missing.conf"), dir.path().join("missing-parent/nginx.conf")] {
            let error = unsafe_error(&dump(&[&path]));
            assert_eq!(error.detail.as_deref(), Some(format!("file not found: {}", path.display()).as_str()));
            assert!(!path.exists());
        }
    }
    #[test]
    fn missing_closure_never_dumps_non_header_contents() {
        for bytes in [vec![], b"events {}\nhttp {}\n# secret-marker-no-header\n".to_vec(), vec![0xff]] {
            let error = unsafe_error(&bytes);
            assert!(!error.detail.unwrap_or_default().contains("secret-marker"));
        }
        let dir = fixture(); let path = config(dir.path(), "nginx.conf");
        let mut output = dump(&[&path]); output.extend_from_slice(&[0xff]); validate(&output).unwrap();
    }
    #[test]
    fn non_utf8_config_contents_are_admitted_without_decoding_them() {
        let dir = fixture(); let path = config(dir.path(), "cp949.conf");
        let bytes = b"# CP949 comment: \xbe\xc8\xb3\xe7\n"; fs::write(&path, bytes).unwrap();
        let mut output = format!("# configuration file {}:\n", path.display()).into_bytes(); output.extend_from_slice(bytes);
        validate(&output).unwrap(); assert_eq!(fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn non_utf8_header_filenames_are_checked_as_os_paths() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let dir = fixture(); let path = dir.path().join(std::ffi::OsString::from_vec(b"include-\xff.conf".to_vec()));
        fs::write(&path, "# fixture\n").unwrap(); fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let mut output = b"# configuration file ".to_vec(); output.extend_from_slice(path.as_os_str().as_bytes()); output.extend_from_slice(b":\n# body\n");
        validate(&output).unwrap();
        let raw = CString::new(path.as_os_str().as_bytes()).unwrap(); assert_eq!(unsafe { libc::chown(raw.as_ptr(), 10001, 10001) }, 0);
        let error = unsafe_error(&output); assert_eq!(error.detail, Some(path.display().to_string()));
    }
    struct RepeatingDump { prefix: std::io::Cursor<Vec<u8>>, padding: usize }
    impl RepeatingDump {
        fn new(prefix: Vec<u8>, total: usize) -> Self {
            assert!(prefix.len() < 8192 && total >= prefix.len());
            let padding = total - prefix.len(); Self { prefix: std::io::Cursor::new(prefix), padding }
        }
    }
    impl std::io::Read for RepeatingDump {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let available = self.fill_buf()?; let n = out.len().min(available.len());
            out[..n].copy_from_slice(&available[..n]); self.consume(n); Ok(n)
        }
    }
    impl BufRead for RepeatingDump {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            if self.prefix.position() < self.prefix.get_ref().len() as u64 { return self.prefix.fill_buf(); }
            static SPACES: [u8; 8192] = [b' '; 8192];
            Ok(&SPACES[..self.padding.min(SPACES.len())])
        }
        fn consume(&mut self, n: usize) {
            if self.prefix.position() < self.prefix.get_ref().len() as u64 { self.prefix.consume(n); }
            else { self.padding -= n; }
        }
    }
    #[test]
    fn stream_cap_boundary_uses_a_repeating_reader_without_a_large_allocation() {
        let dir = fixture(); let path = config(dir.path(), "nginx.conf"); let prefix = dump(&[&path]);
        validate_stream(RepeatingDump::new(prefix.clone(), MAX_NGINX_T_OUTPUT)).unwrap();
        let error = validate_stream(RepeatingDump::new(prefix, MAX_NGINX_T_OUTPUT + 1)).unwrap_err();
        assert_eq!(error.code, "trusted_input_unsafe");
        assert_eq!(error.detail.as_deref(), Some("nginx -T output: output limit exceeded"));
    }
    #[test]
    fn stream_headers_work_across_every_small_buffer_boundary_and_eof() {
        let dir = fixture(); let first = config(dir.path(), "one.conf"); let last = config(dir.path(), "two.conf");
        let mut output = dump(&[&first]);
        output.extend_from_slice(b"# legacy CP949 content: \xbe\xc8\n");
        output.extend_from_slice(format!("# configuration file {}:", last.display()).as_bytes());
        for capacity in [1, 2, 7, HEADER_PREFIX.len(), MAX_HEADER_BYTES] {
            validate_stream(std::io::BufReader::with_capacity(capacity, std::io::Cursor::new(&output))).unwrap();
        }
    }
    #[test]
    fn real_headers_with_nul_or_oversized_paths_fail_closed() {
        let dir = fixture(); let path = config(dir.path(), "nginx.conf"); let good = dump(&[&path]);
        let mut long = HEADER_PREFIX.to_vec(); long.push(b'/'); long.extend(vec![b'x'; MAX_PATH_BYTES]); long.extend_from_slice(b":\n");
        for bad in [b"# configuration file /fixture/nul\0.conf:\n".to_vec(), long] {
            let mut output = good.clone(); output.extend_from_slice(&bad); unsafe_error(&output);
        }
        let mut output = good; output.extend_from_slice(HEADER_PREFIX); output.push(b'/');
        output.extend(vec![b'x'; MAX_PATH_BYTES + 10]); output.extend_from_slice(b" not-a-header\n");
        validate_stream(std::io::BufReader::with_capacity(7, std::io::Cursor::new(output))).unwrap();
    }
    #[test]
    fn stream_read_failure_after_a_valid_header_is_sanitized_and_refused() {
        struct Broken;
        impl std::io::Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> { Err(std::io::Error::other("secret-marker")) }
        }
        let dir = fixture(); let path = config(dir.path(), "nginx.conf");
        let source = std::io::Cursor::new(dump(&[&path])).chain(Broken);
        let error = validate_stream(std::io::BufReader::new(source)).unwrap_err();
        assert_eq!(error.code, "trusted_input_unsafe");
        assert_eq!(error.detail.as_deref(), Some("nginx -T output: failed to read output"));
    }
    #[test]
    fn config_content_is_not_interpreted_or_opened_as_more_paths() {
        let dir = fixture(); let path = config(dir.path(), "nginx.conf");
        let mut output = dump(&[&path]); output.extend_from_slice(b"include \"/not-an-output-header/*.conf\";\n# arbitrary comment\n");
        validate(&output).unwrap();
    }
    #[test]
    fn unsafe_symlink_directory_and_fifo_includes_are_refused_without_reading() {
        let dir = fixture(); let target = config(dir.path(), "target.conf"); let link = dir.path().join("symlink.conf");
        symlink(&target, &link).unwrap(); unsafe_error(&dump(&[&link]));
        unsafe_error(&dump(&[dir.path()]));
        let fifo = dir.path().join("fifo.conf"); let raw = CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0); unsafe_error(&dump(&[&fifo]));
        assert_eq!(fs::read(target).unwrap(), b"# root-owned fixture\n");
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    }
}
