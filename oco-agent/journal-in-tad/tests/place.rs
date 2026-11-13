//! The folder says what it is: a place in the commons' way, bringing the lens of now.

use std::path::PathBuf;

fn folder(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("oco-agent-place-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn read(at: PathBuf) -> toml::Table {
    std::fs::read_to_string(&at)
        .unwrap_or_else(|e| panic!("{}: {e}", at.display()))
        .parse()
        .unwrap()
}

#[test]
fn the_folder_is_a_place_that_brings_the_lens_of_now() {
    let d = folder("made");
    oco_agent_journal_in_tad::InTad::open(&d).unwrap();
    // a mark: which place this is, and where each of its tables is kept - each there to be read
    let mark = read(d.join("oco.toml"));
    let id = mark["project"]["id"].as_str().unwrap();
    // held by the machine, and named for the program whose record it is: a name no owner elsewhere can have
    assert!(
        id.ends_with("/oco-agent") && id.split('/').next().unwrap().contains('.'),
        "{id}"
    );
    assert_eq!(
        (id.split('/').count(), mark["project"]["kind"].as_str()),
        (2, Some("device-lab")),
        "{id}"
    );
    let exports = mark["exports"].as_table().unwrap();
    assert_eq!(
        exports.keys().collect::<Vec<_>>(),
        ["beats", "journal", "said"]
    );
    assert!(exports
        .values()
        .all(|path| d.join(path.as_str().unwrap()).is_file()));
    // a lens: its tables are the declaration itself, and it says what it brings into focus
    let (lens, declared) = (read(d.join("lenses/now.toml")), read(d.join("tables.toml")));
    assert_eq!(lens["lens"]["id"].as_str(), Some("now"));
    assert_eq!(
        lens["tables"], declared["tables"],
        "one declaration: the lens cannot say other than what is kept"
    );
    let parts = lens["focus"].as_array().unwrap();
    assert!(
        parts.len() >= 4
            && parts
                .iter()
                .all(|p| p.get("sql").is_some() && p.get("line").is_some())
    );
    assert!(
        parts[0]["sql"]
            .as_str()
            .unwrap()
            .contains("latest.what = 'claimed'"),
        "what is being done is what was claimed and has not ended"
    );
}

#[test]
fn a_mark_a_person_wrote_stands_and_the_lens_is_brought_up_to_the_adapters() {
    let d = folder("kept");
    std::fs::create_dir_all(d.join("lenses")).unwrap();
    std::fs::write(d.join("oco.toml"), "schema = 1\n# mine\n").unwrap();
    std::fs::write(d.join("lenses/now.toml"), "# an older lens\n").unwrap();
    oco_agent_journal_in_tad::InTad::open(&d).unwrap();
    assert_eq!(
        std::fs::read_to_string(d.join("oco.toml")).unwrap(),
        "schema = 1\n# mine\n"
    );
    assert_eq!(
        read(d.join("lenses/now.toml"))["lens"]["id"].as_str(),
        Some("now")
    );
}
