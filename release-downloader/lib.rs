use mashrl::{
    HTTP::{Headers, ResponseCode},
    make_request,
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

/// returns pairs for names and urls
pub fn get_asset_urls_and_names_from_github_releases(
    owner: &str,
    repository: &str,
    tag: &str,
    pattern: utilities::Pattern<'_>,
    github_token: Option<&str>,
) -> Result<Vec<(String, String)>, BoxedError> {
    let headers = if let Some(token) = github_token {
        Headers::from_string(format!("Authorization: Bearer {token}"))
    } else {
        Headers::empty()
    };

    // TODO should walk pages instead
    const PER_PAGE: u8 = 100;

    // TOOD some releases are included here and so we don't always need to walk
    let release_id: String = {
        let path = if let "latest" = tag {
            format!("repos/{owner}/{repository}/releases/latest")
        } else {
            format!("repos/{owner}/{repository}/releases/tags/{tag}")
        };

        let mut response = make_request("api.github.com", &path, &headers)?;

        if response.code != ResponseCode::OK {
            return Err("could not make request, repository or user may not exist".into());
        }

        let mut body = String::new();
        response.body.read_to_string(&mut body)?;

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

    let path =
        format!("repos/{owner}/{repository}/releases/{release_id}/assets?per_page={PER_PAGE}");
    let mut response = make_request("api.github.com", &path, &headers)?;

    if response.code != ResponseCode::OK {
        return Err("could not make request, repository or user may not exist".into());
    }

    let mut body = String::new();
    response.body.read_to_string(&mut body)?;

    // Relies on fact keys are in order
    let mut download_next_release = false;
    let mut name: &str = "";
    let mut assets = Vec::new();
    let mut asset_idx = 0;

    let _ = parse_json(&body, |keys, value| {
        // eprintln!("{keys:?} to {value:?}");
        if let [JSONKey::Index(idx), key] = keys {
            match key {
                JSONKey::Slice("label") => {
                    let RootJSONValue::String(value) = value else {
                        panic!("expect asset label to be string")
                    };
                    // let origin = value.strip_suffix("gz").unwrap_or(value);
                    // let origin = value.strip_suffix("tar").unwrap_or(value);
                    // let origin = value.strip_suffix("zip").unwrap_or(value);

                    download_next_release = value.contains(OS_MATCHER)
                        && value.contains(ARCH_MATCHER)
                        && pattern.matches(value);

                    // eprintln!("{download_next_release} {value} (pattern={pattern:?})");
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

    eprintln!("Found {asset_idx} assets");

    Ok(assets)
}

pub fn download_from_github(
    url: &str,
    github_token: Option<&str>,
) -> Result<impl Read, BoxedError> {
    let headers = if let Some(token) = github_token {
        Headers::from_string(format!("Authorization: Bearer {token}"))
    } else {
        Headers::empty()
    };

    let actual_asset_url = {
        let url = url
            .strip_prefix("https://github.com")
            .ok_or_else(|| format!("Asset url {url:?} does not start with 'https://github.com'"))?;

        let response = make_request("github.com", url, &headers)?;

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

    let response = make_request(base, url, &headers)?;

    Ok(response.body)
}

#[cfg(feature = "self-update")]
pub fn replace_self(mut content: impl Read) -> Result<(), BoxedError> {
    let temporary_binary_name = "temporary";
    let mut file = File::create(temporary_binary_name)?;

    std::io::copy(&mut content, &mut file)?;

    self_replace::self_replace(temporary_binary_name)?;
    std::fs::remove_file(temporary_binary_name)?;

    Ok(())
}

pub fn write_binary(to: &str, name: &str, mut reader: impl Read) -> Result<(), BoxedError> {
    let p = format!("{to}/{name}");
    let path = Path::new(&p);
    let to = Path::new(to);

    if name.ends_with(".tar.gz") {
        extract_tar_gz(reader, to)?;
    } else if name.ends_with(".tar") {
        extract_tar(reader, to)?;
    } else {
        #[cfg(windows)]
        if name.ends_with(".zip") {
            let mut buffer = Vec::new();
            reader.read_to_end(&mut buffer)?;
            let reader = std::io::Cursor::new(&buffer);
            extract_zip(reader, to)?;
            return Ok(());
        }

        let mut options = File::options();
        options.write(true).truncate(true).read(true);
        let mut file = options.open(path)?;
        std::io::copy(&mut reader, &mut file)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut start = [0; 4];
            file.read_exact(&mut start);
            if b"\x7fELF" == start {
                fs::set_permissions(file, fs::Permissions::from_mode(0o777))?;
            }
        }
    }

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
                value.contains(self.0)
            }
        }
    }
}

pub fn extract_tar(reader: impl Read, output_dir: &Path) -> Result<(), BoxedError> {
    let mut archive = tar::Archive::new(reader);
    archive.unpack(output_dir)?;
    Ok(())
}

pub fn extract_tar_gz(reader: impl Read, output_dir: &Path) -> Result<(), BoxedError> {
    let decompressor = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decompressor);
    archive.unpack(output_dir)?;
    Ok(())
}

#[cfg(windows)]
pub fn extract_zip(reader: impl Read + std::io::Seek, output_dir: &Path) -> Result<(), BoxedError> {
    use std::fs;
    use std::io;
    use zip::ZipArchive;

    let mut archive = ZipArchive::new(reader)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let out_path = output_dir.join(file.mangled_name());

        if file.name().ends_with('/') {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = fs::File::create(&out_path)?;
            io::copy(&mut file, &mut outfile)?;
        }

        // TODO is this needed?
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = file.unix_mode() {
                fs::set_permissions(&out_path, fs::Permissions::from_mode(mode))?;
            }
        }
    }

    Ok(())
}
