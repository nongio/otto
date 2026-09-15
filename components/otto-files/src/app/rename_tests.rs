
use super::rename_selection;

#[test]
fn a_file_selects_the_stem_only() {
    assert_eq!(rename_selection("photo.png", false), 0..5);
}

#[test]
fn a_multi_dot_name_splits_on_the_last_dot() {
    assert_eq!(rename_selection("archive.tar.gz", false), 0..11);
}

#[test]
fn a_directory_selects_the_whole_name() {
    assert_eq!(rename_selection("Documents", true), 0..9);
    assert_eq!(rename_selection("my.folder", true), 0..9);
}

#[test]
fn a_dotfile_selects_the_whole_name() {
    assert_eq!(rename_selection(".bashrc", false), 0..7);
}

#[test]
fn an_extensionless_file_selects_the_whole_name() {
    assert_eq!(rename_selection("README", false), 0..6);
}
