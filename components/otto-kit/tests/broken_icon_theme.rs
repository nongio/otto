//! A theme whose `index.theme` the icon crate cannot parse must cost an icon,
//! not the process.
//!
//! `freedesktop-icons` panics with "Size not found for icon" on a directory
//! section that declares no `Size=`, and real themes ship exactly that. Otto
//! looks icons up from the thread that wants them — the compositor does it
//! while building the dock — so an unguarded lookup turned a malformed theme
//! into a session that died on login.
//!
//! Its own test binary: the lookup crate reads the XDG directories once, so the
//! theme has to be in place before anything else asks it for an icon.

#[test]
fn a_theme_with_a_sizeless_directory_does_not_take_the_process_down() {
    let root = std::env::temp_dir().join(format!("otto-kit-broken-theme-{}", std::process::id()));
    let theme = root.join("icons/Broken");
    let places = theme.join("16x16/places");
    std::fs::create_dir_all(&places).expect("a place to put the theme");
    std::fs::write(
        theme.join("index.theme"),
        "[Icon Theme]\nName=Broken\nDirectories=16x16/places\n\n\
         [16x16/places]\nContext=Places\nMinSize=8\nMaxSize=128\nType=Scalable\n",
    )
    .expect("the index");
    std::fs::write(places.join("user-trash.png"), []).expect("an icon to find");

    // SAFETY: single-threaded, before the first lookup in this process.
    unsafe { std::env::set_var("XDG_DATA_HOME", &root) };

    // Surviving is the first half. The second is that the theme still answers:
    // two bad sections are no reason to repaint the desktop from the default
    // theme, which on a real system does not even have this icon.
    let found = otto_kit::icons::find_icon_in_theme("user-trash", 16, 1, Some("Broken"))
        .expect("the icon is in the theme, malformed section or not");
    assert!(
        found.ends_with("icons/Broken/16x16/places/user-trash.png"),
        "the lookup left the theme it was given: {found}"
    );
    // And the same for the strict form, which has no generic substitution.
    let exact = otto_kit::icons::exact_icon_in_theme("user-trash", 16, Some("Broken"))
        .expect("the icon is in the theme");
    assert!(exact.ends_with("user-trash.png"), "{exact}");

    // A name the theme does not have is a miss, not a wrong file.
    assert!(
        otto_kit::icons::exact_icon_in_theme("no-such-icon-at-all", 16, Some("Broken")).is_none()
    );

    std::fs::remove_dir_all(&root).ok();
}
