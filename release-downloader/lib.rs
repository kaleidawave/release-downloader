use mashrl::{
    HTTP::{Headers, ResponseCode},
    make_get_request,
};
use simple_json_parser::{JSONKey, RootJSONValue, parse as parse_json};

use std::fs::File;
use std::io::Read;
use std::path::Path;

#[cfg(target_os = "windows")]
const OS_MATCHER: &str = "windows";
#[cfg(target_os = "linux")]
const OS_MATCHER: &str = "linux";
#[cfg(target_os = "macos")]
const OS_MATCHER: &str = "macos";

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const ARCH_MATCHER: &str = "x86";
#[cfg(target_arch = "arm")]
const ARCH_MATCHER: &str = "arm";
#[cfg(target_arch = "aarch64")]
const ARCH_MATCHER: &str = "aarch64";

type BoxedError = Box<dyn std::error::Error>;

#[derive(Debug, Clone, Copy)]
pub struct DownloadOptions<'a> {
    pub github_token: Option<&'a str>,
    pub trace: bool,
    pub pattern: utilities::Pattern<'a>,
    pub tag: Option<&'a str>,
    pub match_architecture: bool,
}

impl Default for DownloadOptions<'static> {
    fn default() -> Self {
        DownloadOptions {
            github_token: None,
            trace: false,
            pattern: utilities::Pattern::all(),
            tag: None,
            match_architecture: true,
        }
    }
}

/// returns pairs for names and urls
pub fn get_asset_urls_and_names_from_github_releases(
    owner: &str,
    repository: &str,
    options: DownloadOptions<'_>,
) -> Result<Vec<(String, String)>, BoxedError> {
    let mut headers = Headers::from_iter([
        ("Accept", "application/vnd.github+json"),
        ("X-GitHub-Api-Version", "2022-11-28"),
        ("User-Agent", "kaleidwave/release-downloader"),
    ]);

    if let Some(token) = options.github_token {
        headers.append("Authorization", &format!("Bearer {token}"));
    }

    // TOOD some releases are included here and so we don't always need to walk
    let release_id: String = {
        let path = if let Some(tag) = options.tag {
            format!("repos/{owner}/{repository}/releases/tags/{tag}")
        } else {
            format!("repos/{owner}/{repository}/releases/latest")
        };

        let mut response = make_get_request("api.github.com", &path, &headers)?;

        let mut body = String::new();
        response.body.read_to_string(&mut body)?;

        if response.code != ResponseCode::OK {
            let message = format!(
                "could not make request, repository ({repository}) or user ({owner}) may not exist. recieved {code:?} from 'api.github.com/{path}'. Recieved body ({body:?})",
                code = response.code
            );
            return Err(message.into());
        }

        let mut release_id = "".to_owned();
        let _ = parse_json(&body, |keys, value| {
            if let [JSONKey::Slice("id")] = keys {
                let RootJSONValue::Number(value) = value else {
                    panic!("expect asset label to be string")
                };

                release_id = value.to_owned();

                // TODO break early
            }
        });

        release_id
    };

    // TODO should walk pages instead
    const PER_PAGE: u8 = 100;

    let path =
        format!("repos/{owner}/{repository}/releases/{release_id}/assets?per_page={PER_PAGE}");
    let mut response = make_get_request("api.github.com", &path, &headers)?;

    if response.code != ResponseCode::OK {
        return Err(format!(
            "could not make request for assets. recieved {code:?} from 'api.github.com'",
            code = response.code
        )
        .into());
    }

    let mut body = String::new();
    response.body.read_to_string(&mut body)?;

    // Relies on fact keys are in order
    let mut download_next_release = false;
    let mut name: &str = "";
    let mut assets = Vec::new();
    let mut asset_idx = 0;

    let _ = parse_json(&body, |keys, value| {
        if let [JSONKey::Index(idx), key] = keys {
            match key {
                JSONKey::Slice("label") => {
                    let RootJSONValue::String(value) = value else {
                        panic!("expect asset label to be string")
                    };
                    // let origin = value.strip_suffix("gz").unwrap_or(value);
                    // let origin = value.strip_suffix("tar").unwrap_or(value);
                    // let origin = value.strip_suffix("zip").unwrap_or(value);

                    let name = if value.is_empty() { name } else { value };

                    let architecture = if options.match_architecture {
                        name.contains(OS_MATCHER) && name.contains(ARCH_MATCHER)
                    } else {
                        true
                    };

                    download_next_release = architecture && options.pattern.matches(name);

                    if options.trace {
                        let action = if download_next_release {
                            "downloading"
                        } else {
                            "not downloading"
                        };
                        let pattern = options.pattern;
                        eprintln!("{action} {name:?} (pattern = {pattern:?})");
                    }
                }
                JSONKey::Slice("browser_download_url") => {
                    if download_next_release {
                        let RootJSONValue::String(url) = value else {
                            panic!("expected asset url to be string")
                        };

                        assets.push((name.to_owned(), url.to_owned()));

                        // TODO could exit here
                    }
                }
                JSONKey::Slice("name") => {
                    if let RootJSONValue::String(name2) = value {
                        name = name2;
                    };
                }
                _key => {
                    // eprintln!("{key:?} {value:?}");
                }
            }
            asset_idx = idx + 1;
        }
    });

    if options.trace {
        eprintln!("Scanned {asset_idx} assets");
    }

    Ok(assets)
}

pub fn download_from_github(
    url: &str,
    github_token: Option<&str>,
) -> Result<impl Read, BoxedError> {
    let mut headers = Headers::from_iter([
        // ("Accept", "application/vnd.github+json"),
        // ("X-GitHub-Api-Version", "2022-11-28"),
        ("User-Agent", "kaleidwave/release-downloader"),
    ]);

    if let Some(token) = github_token {
        headers.append("Authorization", &format!("Bearer {token}"));
    }

    let actual_asset_url = {
        let url = url
            .strip_prefix("https://github.com")
            .ok_or_else(|| format!("Asset url {url:?} does not start with 'https://github.com'"))?;

        let response = make_get_request("github.com", url, &headers)?;

        let location = response
            .headers
            .iter()
            .find_map(|(key, value)| (key == "Location").then_some(value.to_owned()));

        location.ok_or("no location")?
    };

    // Finally do download
    let parts = actual_asset_url
        .strip_prefix("https://")
        .and_then(|url| url.split_once('/'));

    let Some((base, url)) = parts else {
        return Err("asset url does not start with 'https://' and have path".into());
    };

    let response = make_get_request(base, url, &headers)?;

    Ok(response.body)
}

pub fn write_binary(
    to: &str,
    name: &str,
    mut reader: impl Read,
    only_binaries: bool,
    trace: bool,
) -> Result<(), BoxedError> {
    let p = format!("{to}/{name}");
    let path = Path::new(&p);
    let to = Path::new(to);

    if name.ends_with(".tar.gz") {
        extract_tar_gz(reader, to, only_binaries, trace)?;
    } else if name.ends_with(".tar") {
        extract_tar(reader, to, only_binaries, trace)?;
    } else if name.ends_with(".zip") {
        #[cfg(windows)]
        {
            // We put into `Cursor` here so reader implements `Seek`
            let mut buffer = Vec::new();
            reader.read_to_end(&mut buffer)?;
            let reader = std::io::Cursor::new(&buffer);
            extract_zip(reader, to, only_binaries, trace)
        }?;

        #[cfg(not(windows))]
        panic!("cannot unzip on `not(windows)`")
    } else {
        let mut options = File::options();
        options.write(true).truncate(true).read(true).create(true);
        let mut file = options.open(path)?;
        std::io::copy(&mut reader, &mut file)?;

        if trace {
            eprintln!("Writing to {path:?}");
        }

        #[cfg(unix)]
        {
            let mut buf = [0; 4];
            if let Ok(_) = file.read_exact(&mut buf)
                && (&buf == b"\x7fELF" || u32::from_le_bytes(start) == 0xFEEDFACFu32)
            {
                let permission =
                    <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o777);
                file.set_permissions(permission)?;
            }
        }
    }

    Ok(())
}

pub fn move_if_binary(
    mut content: impl Read,
    path: &Path,
    only_binaries: bool,
    trace: bool,
) -> Result<(), BoxedError> {
    use std::fs::File;
    use std::io::{Write, copy};

    let mut start = [0; 4];
    // Ignore error here, some files have size < 4
    let _ok = content.read_exact(&mut start);

    let is_binary = &start == b"\x7fELF"
        || start.starts_with(b"MZ")
        || u32::from_le_bytes(start) == 0xFEEDFACFu32;

    if only_binaries && !is_binary {
        return Ok(());
    }

    let mut file = File::create(path)?;

    // Writes bytes that were pulled path
    let _ = file.write(&start)?;
    let _ = copy(&mut content, &mut file)?;

    // Set permissions
    #[cfg(unix)]
    if is_binary {
        let permission =
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o777);
        file.set_permissions(permission)?;
    }

    if trace {
        eprintln!("Writing to {path:?}");
    }

    Ok(())
}

pub fn extract_tar(
    reader: impl Read,
    output_dir: &Path,
    only_binaries: bool,
    trace: bool,
) -> Result<(), BoxedError> {
    let mut archive = tar::Archive::new(reader);
    // archive.set_preserve_permissions(true);
    let entries = archive.entries()?;
    for entry in entries {
        let entry = entry?;
        if let Some(name) = entry.path()?.file_name() {
            let path = output_dir.join(name);
            move_if_binary(entry, &path, only_binaries, trace)?;
        }
    }
    Ok(())
}

pub fn extract_tar_gz(
    reader: impl Read,
    output_dir: &Path,
    only_binaries: bool,
    trace: bool,
) -> Result<(), BoxedError> {
    let decompressor = flate2::read::GzDecoder::new(reader);
    extract_tar(decompressor, output_dir, only_binaries, trace)
}

#[cfg(windows)]
pub fn extract_zip(
    reader: impl Read + std::io::Seek,
    output_dir: &Path,
    only_binaries: bool,
    trace: bool,
) -> Result<(), BoxedError> {
    let mut archive = zip::ZipArchive::new(reader)?;
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        if entry.is_file()
            && let Some(name) = Path::new(entry.name()).file_name()
        {
            let path = output_dir.join(name);
            move_if_binary(entry, &path, only_binaries, trace)?;
        }
    }

    Ok(())
}

/// For updating the current program
#[cfg(feature = "self-update")]
pub fn replace_self(mut content: impl Read) -> Result<(), BoxedError> {
    let temporary_binary_name = "temporary";
    let mut file = File::create(temporary_binary_name)?;

    std::io::copy(&mut content, &mut file)?;

    self_replace::self_replace(temporary_binary_name)?;
    std::fs::remove_file(temporary_binary_name)?;

    Ok(())
}

pub mod utilities {
    #[derive(Debug, Clone, Copy)]
    pub struct Pattern<'a>(&'a str);

    impl<'a> Pattern<'a> {
        pub fn new(pattern: &'a str) -> Self {
            Self(pattern)
        }

        pub fn all() -> Self {
            Self("*")
        }

        // TODO temp
        pub fn matches(&self, value: &str) -> bool {
            if let "*" = self.0 {
                true
            } else {
                for part in self.0.split('|') {
                    if value.contains(part) {
                        return true;
                    }
                }
                false
            }
        }
    }
}
