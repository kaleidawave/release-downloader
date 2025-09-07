use std::env;
use std::fs;

use release_downloader::{
    download_from_github, get_asset_urls_and_names_from_github_releases, utilities::Pattern,
    write_binary,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);

    let first_argument = arguments.next();
    let endpoint = first_argument.as_deref().unwrap_or("help");

    match endpoint {
        "info" => {
            let run_id = option_env!("GITHUB_RUN_ID");
            let date = option_env!("GIT_LAST_COMMIT").unwrap_or_default();
            let after = run_id
                .map(|commit| format!(" (commit {commit} {date})"))
                .unwrap_or_default();

            eprintln!(
                "release-downloader{after} (powered by 'mashrl': minimal-and-simple-http-request-library)"
            );
            eprintln!(
                "find releases with 'list' otherwise download binaries from comma-separated list of 'owner/repository@tag[name]' location"
            );
        }
        "list" => {
            todo!("print options");
        }
        argument => {
            let github_token =
                env::vars().find_map(|(name, value)| (name == "GH_TOKEN").then_some(value));

            let home = env::vars().find_map(|(name, value)| (name == "HOME").then_some(value));

            let mut trace = false;
            let mut specified_destination = None;

            // WIP
            for argument in arguments {
                if let "--trace" = argument.as_str() {
                    trace = true;
                } else {
                    specified_destination = Some(argument);
                }
            }

            let destination = if let Some(path) = specified_destination {
                path
            } else if let Some(home) = home {
                #[cfg(unix)]
                {
                    format!("{home}/.local/bin")
                }
                #[cfg(not(unix))]
                {
                    format!("{home}/.tools")
                }
            } else {
                // TODO ?
                ".".to_owned()
            };

            fs::create_dir_all(&destination)?;

            let github_token = github_token.as_deref();

            for argument in argument.split(',').map(str::trim) {
                let (owner, rest) = argument.split_once('/').unwrap();
                let (repository, rest) = rest.split_once(['@', '[']).unwrap_or((rest, "latest"));
                let (tag, rest) = utilities::split_once_inclusive(rest, '[').unwrap_or((rest, ""));

                let pattern = if let Some(pattern) = utilities::split_surrounding(rest, ('[', ']'))
                {
                    Pattern::new(pattern)
                } else {
                    Pattern::new(repository)
                };

                let out = get_asset_urls_and_names_from_github_releases(
                    owner,
                    repository,
                    tag,
                    pattern,
                    github_token,
                    trace,
                )?;

                if out.is_empty() {
                    eprintln!("no assets matching {pattern:?}");
                }

                for (name, url) in out.into_iter() {
                    if trace {
                        eprintln!("Downloading {url:?}");
                    }
                    let content = download_from_github(&url, github_token)?;
                    write_binary(&destination, &name, content)?;
                }
            }
        }
    }
    Ok(())
}

mod utilities {
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
