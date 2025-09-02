use std::env;
use std::fs;

use release_downloader::{
    download_from_github, get_asset_urls_and_names_from_github_releases, utilities::Pattern,
    write_binary,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
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

            let github_token = github_token.as_deref();

            for argument in first_argument.split(',') {
                let (owner, rest) = argument.split_once('/').unwrap();
                let (repository, rest) = rest.split_once(['@', '[']).unwrap_or((rest, "latest[*]"));
                let (tag, rest) =
                    utilities::split_once_inclusive(rest, '[').unwrap_or((rest, "[*]"));
                let pattern = utilities::split_surrounding(rest, ('[', ']'))
                    .map(Pattern::new)
                    .unwrap_or(Pattern::all());

                let out = get_asset_urls_and_names_from_github_releases(
                    owner,
                    repository,
                    tag,
                    pattern,
                    github_token,
                )?;

                if out.is_empty() {
                    eprintln!("no assets matching {pattern:?}");
                }

                for (name, url) in out.into_iter() {
                    eprintln!("Downloading {url:?}");
                    let content = download_from_github(&url, github_token)?;
                    write_binary(&destination, &name, content)?;
                }
            }
        }
    } else {
        println!("expected argument");
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
