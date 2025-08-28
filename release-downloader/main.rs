use std::env;
use std::fs;
use std::path::Path;

use mashrl::{
    HTTP::{Headers, ResponseCode},
    make_request,
};
use simple_json_parser::{JSONKey, RootJSONValue, parse as parse_json};

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

type ProcessResult = Result<(), Box<dyn std::error::Error>>;

fn main() -> ProcessResult {
    let first_argument = env::args().nth(1);

    if let Some(first_argument) = first_argument {
        if let "list" = first_argument.as_str() {
            todo!("print options");
        } else {
            let github_token =
                env::vars().find_map(|(name, value)| (name == "GH_TOKEN").then_some(value));

            let home = env::vars().find_map(|(name, value)| (name == "HOME").then_some(value));

            let destination = if let Some(home) = home {
                format!("{home}/.local/bin")
            } else if let Some(path) = env::args().nth(2) {
                path
            } else {
                // TODO ?
                ".".to_owned()
            };

            fs::create_dir_all(&destination)?;

            let headers = if let Some(token) = github_token {
                Headers::from_string(format!("Authorization: Bearer {token}"))
            } else {
                Headers::empty()
            };

            for argument in first_argument.split(',') {
                let (owner, rest) = argument.split_once('/').unwrap();
                let (repository, rest) = rest.split_once(['@', '[']).unwrap_or((rest, "latest[*]"));
                let (tag, rest) =
                    utilities::split_once_inclusive(rest, '[').unwrap_or((rest, "[*]"));
                let pattern = utilities::split_surrounding(rest, ('[', ']'))
                    .map(utilities::Pattern::new)
                    .unwrap_or(utilities::Pattern::all());

                let out = get_asset_urls_and_names(owner, repository, tag, pattern, &headers)?;

                for (name, url) in out.into_iter() {
                    eprintln!("Downloading {url:?}");
                    download(&name, &url, &destination, &headers)?;
                }
            }
        }
    } else {
        println!("expected argument");
    }
    Ok(())
}

fn get_asset_urls_and_names(
    owner: &str,
    repository: &str,
    tag: &str,
    _pattern: utilities::Pattern<'_>,
    headers: &Headers<'_>,
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let path = if let "latest" = tag {
        format!("repos/{owner}/{repository}/releases/latest")
    } else {
        format!("repos/{owner}/{repository}/releases/tags/{tag}")
    };

    let response = make_request("api.github.com", &path, &headers)?;

    if response.code != ResponseCode::OK {
        return Err(format!("could not make request, repository or user may not exist").into());
    }

    // Relies on fact keys are in order
    let mut download_next_release = false;
    let mut name: &str = "";

    let mut assets = Vec::new();

    let _ = parse_json(str::from_utf8(&response.body).unwrap(), |keys, value| {
        if let [JSONKey::Slice("assets"), JSONKey::Index(_), key] = keys {
            match key {
                JSONKey::Slice("label") => {
                    let RootJSONValue::String(value) = value else {
                        panic!("expect asset label to be string")
                    };
                    // let origin = value.strip_suffix("gz").unwrap_or(value);
                    // let origin = value.strip_suffix("tar").unwrap_or(value);
                    // let origin = value.strip_suffix("zip").unwrap_or(value);

                    download_next_release =
                        value.contains(OS_MATCHER) && value.contains(ARCH_MATCHER);
                    // TODO pattern
                }
                JSONKey::Slice("browser_download_url") => {
                    if download_next_release {
                        let RootJSONValue::String(url) = value else {
                            panic!("expected asset url to be string")
                        };

                        // TODO temp
                        eprintln!("downloading {name} ({url})");

                        assets.push((name.to_owned(), url.to_owned()));
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
        }
    });

    Ok(assets)
}

fn download(name: &str, url: &str, to: &str, headers: &Headers<'_>) -> ProcessResult {
    use std::io::{BufWriter, Write};

    let actual_asset_url = {
        let url = url
            .strip_prefix("https://github.com")
            .ok_or_else(|| format!("Asset url {url:?} does not start with 'https://github.com'"))?;

        let response = make_request("github.com", url, headers)?;

        let mut location = None;
        for (header, value) in &response.headers {
            if let "Location" = header {
                location = Some(value.to_owned());
            }
        }

        location.ok_or("no location")?
    };

    // Finally do download
    let (base, url) = actual_asset_url
        .strip_prefix("https://")
        .unwrap()
        .split_once('/')
        .unwrap();

    let response = make_request(base, url, headers)?;

    let p = format!("{to}/{name}");
    let path = std::path::Path::new(&p);
    let file = fs::File::create(path)?;
    let mut writer = BufWriter::new(file);

    #[allow(unused)]
    let is_elf_binary = response.body.starts_with(b"\x7fELF");

    writer.write_all(&response.body)?;

    if name.ends_with(".tar.gz") {
        extract_tar_gz(path, std::path::Path::new(to))?;
    } else if name.ends_with(".tar") {
        extract_tar(path, std::path::Path::new(to))?;
    } else {
        #[cfg(windows)]
        if name.ends_with(".zip") {
            extract_zip(path, std::path::Path::new(to))?;
        }

        #[cfg(unix)]
        if is_elf_binary {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(path, fs::Permissions::from_mode(0o777))?;
        }
    }

    Ok(())
}

#[allow(dead_code)]
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
    }

    pub fn split_once_inclusive(on: &str, chr: char) -> Option<(&str, &str)> {
        on.find(chr).map(|idx| on.split_at(idx))
    }

    pub fn split_surrounding(on: &str, (before, after): (char, char)) -> Option<&str> {
        if let Some(on) = on.strip_prefix(before) {
            on.strip_suffix(after)
        } else {
            None
        }
    }
}

fn extract_tar(path: &Path, output_dir: &Path) -> ProcessResult {
    let file = fs::File::open(path)?;
    let mut archive = tar::Archive::new(file);
    archive.unpack(output_dir)?;
    Ok(())
}

fn extract_tar_gz(path: &Path, output_dir: &Path) -> ProcessResult {
    let file = fs::File::open(path)?;
    let decompressor = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decompressor);
    archive.unpack(output_dir)?;
    Ok(())
}

#[cfg(windows)]
fn extract_zip(path: &Path, output_dir: &Path) -> ProcessResult {
    use std::io;
    use zip::ZipArchive;

    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
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
