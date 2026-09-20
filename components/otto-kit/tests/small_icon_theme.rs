//! A theme that stops below the size the dock asks for must answer with its
//! largest icon, not its smallest.
//!
//! `freedesktop-icons` ranks its closest-size fallback by a signed distance,
//! so for the fixed-size directories most themes are built from every
//! directory scores negative on an oversized request and the 16px one wins.
//! The dock asks at 512 on purpose, and a theme topping out at 256 used to
//! come back as a blown-up 16px smear.
//!
//! Its own test binary: the lookup crate reads the XDG directories once, so
//! the theme has to be in place before anything else asks it for an icon.

#[test]
fn a_theme_that_stops_short_answers_with_its_largest_icon() {
    let root = std::env::temp_dir().join(format!("otto-kit-small-theme-{}", std::process::id()));
    let theme = root.join("icons/Small");
    let sizes = [16, 64, 256];
    for size in sizes {
        std::fs::create_dir_all(theme.join(format!("apps/{size}"))).expect("a place for the theme");
        std::fs::write(theme.join(format!("apps/{size}/otto-probe.png")), [])
            .expect("an icon to find");
    }
    // Only the small directory has this one: a theme may stop short per icon.
    std::fs::write(theme.join("apps/16/otto-tiny.png"), []).expect("an icon to find");
    let directories = sizes.map(|s| format!("apps/{s}")).join(",");
    let sections: String = sizes
        .iter()
        .map(|s| format!("\n[apps/{s}]\nSize={s}\nContext=Applications\nType=Fixed\n"))
        .collect();
    std::fs::write(
        theme.join("index.theme"),
        format!("[Icon Theme]\nName=Small\nDirectories={directories}\n{sections}"),
    )
    .expect("the index");

    // SAFETY: single-threaded, before the first lookup in this process.
    unsafe { std::env::set_var("XDG_DATA_HOME", &root) };

    let found = otto_kit::icons::exact_icon_in_theme("otto-probe", 512, Some("Small"))
        .expect("the icon is in the theme");
    assert!(
        found.ends_with("apps/256/otto-probe.png"),
        "a request above the theme's largest size should land on it: {found}"
    );

    // A size the theme does have is still answered exactly.
    let exact = otto_kit::icons::exact_icon_in_theme("otto-probe", 64, Some("Small"))
        .expect("the icon is in the theme");
    assert!(exact.ends_with("apps/64/otto-probe.png"), "{exact}");

    // And an icon that only exists small is still found.
    let tiny = otto_kit::icons::exact_icon_in_theme("otto-tiny", 512, Some("Small"))
        .expect("the icon is in the theme");
    assert!(tiny.ends_with("apps/16/otto-tiny.png"), "{tiny}");

    std::fs::remove_dir_all(&root).ok();
}
