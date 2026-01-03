use std::env;
use std::fs;
use std::path::PathBuf;

use release_downloader::{
    download_from_github, get_asset_urls_and_names_from_github_releases, pattern, write_binary,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);

    let first_argument = arguments.next();
    let endpoint = first_argument.as_deref().unwrap_or("--help");

    match endpoint {
        "info" | "--help" => {
            let run_id = option_env!("GITHUB_RUN_ID");
            let date = option_env!("GIT_LAST_COMMIT").unwrap_or_default();
            let after = run_id
                .map(|commit| format!(" (commit {commit} {date})"))
                .unwrap_or_default();

            eprintln!(
                "release-downloader{after} (powered by 'mashrl': minimal-and-simple-http-request-library)"
            );
            eprintln!(
                "download binaries from comma-separated list of 'owner/repository@tag[name]' location"
            );
            eprintln!("run with '--trace' to show intermediate findings");
            // eprintln!("release-downloader info shows information about releases")
        }
        "list" => {
            let owner = arguments.next().expect("owner");
            let repository = arguments.next().expect("repository");
            let tag = arguments.next();
            let options = release_downloader::DownloadOptions {
                tag: tag.as_deref(),
                pattern: pattern::Pattern::all(),
                github_token: None,
                trace: false,
                match_architecture: false,
            };
            let items = release_downloader::get_statistics(&owner, &repository, options)?;
            for (name, count) in items {
                println!("{name}: {count} download(s)");
            }
        }
        argument => {
            let github_token =
                env::vars().find_map(|(name, value)| (name == "GH_TOKEN").then_some(value));
            let github_token = github_token.as_deref();

            let mut trace: bool = false;
            let mut match_architecture: bool = true;
            let mut only_binaries: bool = true;
            let mut specified_destination: Option<String> = None;

            for option in arguments {
                if "--trace" == option {
                    trace = true;
                } else if "--ignore-architecture" == option {
                    match_architecture = false;
                } else if "--all-assets" == option {
                    only_binaries = false;
                } else {
                    specified_destination = Some(option);
                }
            }

            let destination: PathBuf = if let Some(path) = specified_destination {
                PathBuf::from(path)
            } else if let Some(home) = env::home_dir() {
                if cfg!(unix) {
                    home.join(".local").join("bin")
                } else {
                    home.join(".tools")
                }
            } else {
                env::current_dir().unwrap()
            };

            fs::create_dir_all(&destination)?;

            // FUTURE could be better
            let destination = destination.display().to_string();

            for argument in argument.split(',').map(str::trim) {
                let (owner, repository, tag, pattern) =
                    input::parse_specifiers_and_pattern(argument);

                let options = release_downloader::DownloadOptions {
                    tag,
                    pattern,
                    github_token,
                    trace,
                    match_architecture,
                };

                let out =
                    get_asset_urls_and_names_from_github_releases(owner, repository, options)?;

                if out.is_empty() {
                    if match_architecture {
                        let specifier = release_downloader::get_architecture_specifier();
                        eprintln!("no assets matching {pattern:?} for architecture {specifier:?}");
                    } else {
                        eprintln!("no assets matching {pattern:?}");
                    }
                }

                for (name, url) in out {
                    if trace {
                        eprintln!("Downloading {url:?}");
                    }
                    let content = download_from_github(&url, github_token)?;
                    write_binary(&destination, &name, content, only_binaries, trace)?;
                }
            }
        }
    }
    Ok(())
}

mod input {
    use release_downloader::pattern::Pattern;

    pub fn split_once_inclusive<'a>(on: &'a str, chr: &[char]) -> Option<(&'a str, &'a str)> {
        on.find(chr).map(|idx| on.split_at(idx))
    }

    pub fn split_surrounding(on: &str, (before, after): (char, char)) -> Option<&str> {
        if let Some(on) = on.strip_prefix(before) {
            on.strip_suffix(after)
        } else {
            None
        }
    }

    pub fn parse_specifiers_and_pattern(on: &str) -> (&str, &str, Option<&str>, Pattern<'_>) {
        let (owner, rest) = on.split_once('/').unwrap();
        let (repository, rest) = split_once_inclusive(rest, &['@', '[']).unwrap_or((rest, ""));
        let (tag, rest) = if let Some(after) = rest.strip_prefix('@') {
            let (tag, rest) = split_once_inclusive(after, &['[']).unwrap_or((after, ""));
            (Some(tag), rest)
        } else {
            (None, rest)
        };
        let rest = split_surrounding(rest, ('[', ']'));
        let pattern = if let Some(pattern) = rest {
            Pattern::new(pattern)
        } else {
            Pattern::all()
        };

        (owner, repository, tag, pattern)
    }

    #[cfg(test)]
    mod tests {
        use super::{Pattern, parse_specifiers_and_pattern};

        #[test]
        fn full() {
            let out = parse_specifiers_and_pattern("a/b@c[d]");
            assert_eq!(out, ("a", "b", Some("c"), Pattern::new("d")));
        }

        #[test]
        fn elided_tag() {
            let out = parse_specifiers_and_pattern("a/b[d]");
            assert_eq!(out, ("a", "b", None, Pattern::new("d")));
        }

        #[test]
        fn elided_specifier() {
            let out = parse_specifiers_and_pattern("a/b@c");
            assert_eq!(out, ("a", "b", Some("c"), Pattern::all()));
        }

        #[test]
        fn elided_specifier_and_tag() {
            let out = parse_specifiers_and_pattern("a/b");
            assert_eq!(out, ("a", "b", None, Pattern::all()));
        }
    }
}
