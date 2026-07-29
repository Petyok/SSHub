use sshub::metadata::{HostMetadata, MetadataDb, MetadataStore};
use tempfile::NamedTempFile;

#[test]
fn upsert_persists_across_reopen() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();

    let meta = HostMetadata {
        host_name: "web".into(),
        tags: vec!["prod".into(), "db".into()],
        description: Some("Primary web server".into()),
        environment: Some("prod".into()),
        favorite: true,
        last_connected: Some(1_700_000_000),
        ..Default::default()
    };

    {
        let db = MetadataDb::open(path).unwrap();
        db.upsert(&meta).unwrap();
    }

    {
        let db = MetadataDb::open(path).unwrap();
        let loaded = db.get("web").unwrap().expect("host metadata row");
        assert_eq!(loaded, meta);
    }
}

#[test]
fn ensure_defaults_and_mutations_persist() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path();

    {
        let db = MetadataDb::open(path).unwrap();
        db.ensure_defaults(&["alpha".into(), "beta".into()])
            .unwrap();
        db.toggle_favorite("alpha").unwrap();
        db.set_last_connected("beta", 42).unwrap();
    }

    {
        let db = MetadataDb::open(path).unwrap();
        assert!(db.get("alpha").unwrap().unwrap().favorite);
        assert_eq!(db.get("beta").unwrap().unwrap().last_connected, Some(42));
    }
}
