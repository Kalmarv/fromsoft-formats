//! `Binder::member` finds a member by the file name its path ends with, ASCII case folded.
use fromsoft_formats::{Binder, BinderFile};

fn file(id: i32, name: Option<&str>) -> BinderFile {
    BinderFile {
        id: Some(id),
        name: name.map(str::to_owned),
        flags: 0,
        bytes: vec![id as u8],
    }
}

#[test]
fn a_member_is_found_by_its_file_name_in_any_case() {
    let binder = Binder {
        version: [0; 8],
        format: 0,
        files: vec![
            file(1, None),
            file(2, Some(r"N:\GR\data\Param\NpcParam.PARAM")),
            file(3, Some("param/other.param")),
            file(4, Some("npcparam.param")),
        ],
    };
    // the first with the name, whatever the directory and the case
    assert_eq!(binder.member("npcparam.param").map(|f| f.id), Some(Some(2)));
    assert_eq!(binder.member("OTHER.PARAM").map(|f| f.id), Some(Some(3)));
    // not a substring, not a path, not a member without a name
    assert!(binder.member("param").is_none());
    assert!(binder.member("param/other.param").is_none());
    assert!(binder.member("").is_none());
}
