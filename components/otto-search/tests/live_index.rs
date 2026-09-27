//! Against the real index, which is the only thing that can say whether the
//! SPARQL is *accepted* as well as well-formed. Ignored by default: it needs a
//! running LocalSearch, and what it finds depends on the machine.
//!
//!     cargo test -p otto-search --test live_index -- --ignored --nocapture

// Rust guideline compliant 2026-02-21

use std::path::PathBuf;
use std::time::Instant;

use otto_search::{index, parse, Clock, Plan, Scope};

/// How long one query may take before the shape is considered wrong. The
/// slow shapes this guards against took tens of seconds, not two.
const BUDGET_SECS: f64 = 5.0;

#[test]
#[ignore = "needs a running LocalSearch indexer"]
fn every_filter_is_accepted_and_fast() {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    let scope = Scope {
        roots: vec![home.clone()],
        home: Some(home.clone()),
        cwd: Some(home),
    };
    for text in [
        "rs",
        "otfl",
        "\"read me\"",
        "-kind:folder sort:modified",
        "kind:image,video modified:<30d",
        "kind:document text:invoice",
        "kind:app",
        "kind:archive size:>10M sort:size",
        "text:\"tax return\"",
        "report -text:draft",
        "modified:2025 -in:~/.cache",
        "in:~/Documents kind:pdf",
    ] {
        let plan = Plan::new(&parse(text), &scope, Clock::now()).expect("answerable");
        let sparql = plan.sparql(1000);
        let started = Instant::now();
        let hits =
            index::search(&sparql).unwrap_or_else(|e| panic!("{text}: {e}\n{}", sparql.text));
        let took = started.elapsed().as_secs_f64();
        let snippet = hits
            .iter()
            .find_map(|h| h.snippet.as_ref())
            .map(ToString::to_string);
        println!(
            "{text:40} {:5} hits {took:5.2}s {}",
            hits.len(),
            snippet.unwrap_or_default()
        );
        assert!(
            took < BUDGET_SECS,
            "{text} took {took:.1}s:\n{}",
            sparql.text
        );
        if sparql.snippets {
            assert!(
                hits.iter().all(|h| h.snippet.is_some()),
                "{text}: a hit without its snippet"
            );
        }
    }
}

#[test]
#[ignore = "needs a running LocalSearch indexer"]
fn counts_are_answered_quickly() {
    let started = Instant::now();
    let counts = index::counts().expect("counts");
    let took = started.elapsed().as_secs_f64();
    println!("{counts:?} in {took:.2}s");
    assert!(counts.files > 0 && counts.folders > 0);
    assert!(took < BUDGET_SECS, "counting took {took:.1}s");
}

/// The driver end to end: every result is a file on disk now, within the
/// limit, one per path, and Recent's order is newest first.
#[test]
#[ignore = "needs a running LocalSearch indexer"]
fn find_returns_real_files_within_the_limit() {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    let scope = Scope {
        roots: vec![home.clone()],
        home: Some(home.clone()),
        cwd: Some(home),
    };
    for (text, limit) in [("-kind:folder sort:modified", 20), ("rs", 500)] {
        let plan = Plan::new(&parse(text), &scope, Clock::now()).expect("answerable");
        let started = Instant::now();
        let best = otto_search::find(&plan, limit, &mut ())
            .expect("the index answered")
            .best();
        println!(
            "{text:30} {:4} found {:5.2}s",
            best.len(),
            started.elapsed().as_secs_f64()
        );
        assert!(!best.is_empty() && best.len() <= limit, "{text}");
        assert!(best.iter().all(|f| f.path.symlink_metadata().is_ok()));
        let mut paths: Vec<_> = best.iter().map(|f| &f.path).collect();
        paths.dedup();
        assert_eq!(paths.len(), best.len(), "{text}: a path twice");
        if text.contains("sort:modified") {
            assert!(best.windows(2).all(|w| w[0].modified >= w[1].modified));
        }
    }
}
