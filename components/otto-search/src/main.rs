//! `otto-search`: find files from the command line, in the same language as
//! the Files search strip. See `specs/search-language.md`.

// Rust guideline compliant 2026-02-21

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use otto_search::{find, index, parse, Clock, Plan, Scope};

const USAGE: &str = "\
usage: otto-search [--in DIR]... [--limit N] [--json] QUERY...

Find files through the desktop's file index. The words of QUERY are joined
with spaces and read as one search, in the language Files uses:

  invoice tax            names containing both words (letters in order)
  \"tax return\"           a name containing this exact text
  text:ricevuta          files whose contents mention it
  kind:pdf               document pdf image video audio text archive app folder
  in:~/Documents         under that folder (relative to the current one)
  modified:<7d           today yesterday week month year <7d >1y 2025-03
  size:>100M             K M G T, powers of 1024
  sort:modified          relevance modified size name
  -draft  -kind:image    leave out

Options:
  --in DIR     where to look when the query has no in: (default: home);
               repeat for several folders
  --limit N    at most N results (default 20)
  --json       one JSON object per line: path, name, kind, modified, size,
               snippet; then a last line, index: state, progress and
               remaining_seconds, saying what the file indexer is doing
  -h, --help   this text

Exit status: 0 found something, 1 found nothing, 2 the file index is not
running, 3 a usage error.";

/// Results printed when `--limit` is not given: enough to pick from, few
/// enough to read.
const DEFAULT_LIMIT: usize = 20;

/// Exit statuses, as the usage text lists them.
const FOUND_NOTHING: u8 = 1;
const UNAVAILABLE: u8 = 2;
const USAGE_ERROR: u8 = 3;

/// What the command line asked for.
#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    query: String,
    roots: Vec<PathBuf>,
    limit: Option<usize>,
    json: bool,
    help: bool,
}

/// Read the arguments after the program name.
///
/// Only the options above are options. Any other word, including one that
/// starts with `-` such as `-kind:image`, is part of the query; `--` ends the
/// options for a query word that would otherwise read as one.
fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut words = Vec::new();
    let mut args = args.into_iter();
    let mut options = true;
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy().into_owned();
        if !options {
            words.push(text);
            continue;
        }
        let (flag, inline) = match text.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag.to_owned(), Some(value)),
            _ => (text.clone(), None),
        };
        let mut value = |name: &str| -> Result<OsString, String> {
            match inline {
                Some(v) => Ok(OsString::from(v)),
                None => args.next().ok_or_else(|| format!("{name} needs a value")),
            }
        };
        match flag.as_str() {
            "--" => options = false,
            "-h" | "--help" => parsed.help = true,
            "--json" => parsed.json = true,
            "--in" => parsed.roots.push(PathBuf::from(value("--in")?)),
            "--limit" => {
                let text = value("--limit")?;
                let limit = text
                    .to_str()
                    .and_then(|t| t.parse::<usize>().ok())
                    .filter(|&n| n > 0)
                    .ok_or_else(|| format!("--limit wants a positive number, not {text:?}"))?;
                parsed.limit = Some(limit);
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            _ => words.push(text),
        }
    }
    parsed.query = words.join(" ");
    Ok(parsed)
}

/// `path` made absolute: `~` against `home`, anything relative against `cwd`.
fn resolve(path: &Path, home: Option<&Path>, cwd: Option<&Path>) -> PathBuf {
    if let (Ok(rest), Some(home)) = (path.strip_prefix("~"), home) {
        return home.join(rest);
    }
    match cwd {
        Some(cwd) if path.is_relative() => cwd.join(path),
        _ => path.to_path_buf(),
    }
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(why) => {
            eprintln!("otto-search: {why}\n\n{USAGE}");
            return ExitCode::from(USAGE_ERROR);
        }
    };
    if args.help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.query.trim().is_empty() {
        eprintln!("otto-search: nothing to search for\n\n{USAGE}");
        return ExitCode::from(USAGE_ERROR);
    }

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let cwd = std::env::current_dir().ok();
    let roots = if args.roots.is_empty() {
        home.iter().cloned().collect()
    } else {
        args.roots
            .iter()
            .map(|root| resolve(root, home.as_deref(), cwd.as_deref()))
            .collect()
    };

    let query = parse(&args.query);
    for (span, issue) in query.issues() {
        let token = args.query.get(span.clone()).unwrap_or_default();
        eprintln!("otto-search: hint: {issue}; searched for {token:?} as a word");
    }

    let scope = Scope { roots, home, cwd };
    let Some(plan) = Plan::new(&query, &scope, Clock::now()) else {
        eprintln!("otto-search: nowhere to look: HOME is not set; pass --in DIR");
        return ExitCode::from(USAGE_ERROR);
    };

    let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
    let found = find(&plan, limit, &mut ());
    // Asked after the search, so it says whether the answer just given may be
    // missing files the indexer has not reached.
    let status = index::status();
    // With --json the status is the last line, after every file and on every
    // way out, so a reader taking one file per line always finds it at the end.
    let status_line = || {
        if args.json {
            println!("{}", status.to_json());
        }
    };
    if !args.json && status.is_behind() {
        eprintln!(
            "otto-search: note: the file index is {} ({:.0}% done); results may be incomplete",
            status.state.as_str(),
            status.progress * 100.0
        );
    }
    let best = match found {
        Ok(mut results) => results.best(),
        Err(why) => {
            eprintln!(
                "otto-search: {why}\n\
                 The file indexer is not running. Install LocalSearch (the \
                 localsearch package) and start it, for example with \
                 systemctl --user start localsearch-3"
            );
            status_line();
            return ExitCode::from(UNAVAILABLE);
        }
    };

    let mut out = std::io::stdout().lock();
    for found in &best {
        let written = if args.json {
            writeln!(out, "{}", found.to_json())
        } else {
            use std::os::unix::ffi::OsStrExt as _;
            out.write_all(found.path.as_os_str().as_bytes())
                .and_then(|()| out.write_all(b"\n"))
        };
        // A closed pipe (`| head`) is the reader having had enough.
        if written.is_err() {
            break;
        }
    }
    if args.json {
        let _ = writeln!(out, "{}", status.to_json());
    }
    let _ = out.flush();

    if best.is_empty() {
        ExitCode::from(FOUND_NOTHING)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        parse_args(list.iter().map(OsString::from))
    }

    #[test]
    fn words_join_and_exclusions_stay_in_the_query() {
        let parsed = args(&["invoice", "-kind:image", "modified:<7d"]).unwrap();
        assert_eq!(parsed.query, "invoice -kind:image modified:<7d");
        assert_eq!(parsed.limit, None);
        assert!(!parsed.json);
    }

    #[test]
    fn options_in_either_spelling_and_anywhere() {
        let parsed = args(&[
            "--json",
            "tax",
            "--in",
            "~/Documents",
            "--in=notes",
            "--limit=5",
            "return",
        ])
        .unwrap();
        assert_eq!(parsed.query, "tax return");
        assert_eq!(
            parsed.roots,
            [PathBuf::from("~/Documents"), PathBuf::from("notes")]
        );
        assert_eq!(parsed.limit, Some(5));
        assert!(parsed.json);
    }

    #[test]
    fn a_double_dash_ends_the_options() {
        let parsed = args(&["--", "--json", "x"]).unwrap();
        assert_eq!(parsed.query, "--json x");
        assert!(!parsed.json);
    }

    #[test]
    fn mistakes_are_usage_errors() {
        assert!(args(&["--in"]).is_err());
        assert!(args(&["--limit", "0", "x"]).is_err());
        assert!(args(&["--limit", "many", "x"]).is_err());
        assert!(args(&["--frobnicate", "x"]).is_err());
        assert!(args(&["-h"]).unwrap().help);
    }

    #[test]
    fn roots_resolve_against_home_and_the_current_folder() {
        let home = Path::new("/home/u");
        let cwd = Path::new("/home/u/work");
        let at = |p: &str| resolve(Path::new(p), Some(home), Some(cwd));
        assert_eq!(at("~"), PathBuf::from("/home/u"));
        assert_eq!(at("~/Documents"), PathBuf::from("/home/u/Documents"));
        assert_eq!(at("notes"), PathBuf::from("/home/u/work/notes"));
        assert_eq!(at("/mnt/data"), PathBuf::from("/mnt/data"));
    }
}
