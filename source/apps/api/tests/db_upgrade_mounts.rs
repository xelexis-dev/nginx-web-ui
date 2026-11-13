#[path="../src/db_upgrade/error.rs"] mod error;
#[path="../src/db_upgrade/mounts.rs"] mod mounts;
use std::path::Path;
const MOUNTS:&str="1 0 0:9 / / rw - overlay overlay rw\n2 1 8:1 /vol/old /source/db ro - ext4 /dev/vda rw\n3 1 8:1 /vol/app /source/app ro - ext4 /dev/vda rw\n4 1 8:1 /vol/new /destination/db rw - ext4 /dev/vda rw\n5 1 8:1 /vol/new-app /destination/app rw - ext4 /dev/vda rw\n";
#[test]
fn kernel_roots_identify_bind_aliases_hidden_by_distinct_container_paths() {
    let table=mounts::Table::parse(MOUNTS).unwrap();
    let roots=[Path::new("/source/db"),Path::new("/source/app"),Path::new("/destination/db"),Path::new("/destination/app")];
    table.distinct(&roots).unwrap();table.require_mount(roots[0],true).unwrap();table.require_mount(roots[2],false).unwrap();
    for physical in ["/vol/old","/vol/old/child"] {
        let altered=MOUNTS.replace("/vol/new /destination/db",&format!("{physical} /destination/db"));
        assert_eq!(mounts::Table::parse(&altered).unwrap().distinct(&roots).unwrap_err().code,"unsupported_layout");
    }
    assert!(table.require_mount(Path::new("/source/db/subdir"),true).is_err());
}
#[test]
fn nested_mounts_readwrite_sources_and_malformed_kernel_records_refuse() {
    let roots=[Path::new("/source/db"),Path::new("/destination/db")];
    let nested=format!("{MOUNTS}6 2 8:2 / /source/db/pg_wal ro - ext4 /dev/vdb rw\n");
    assert!(mounts::Table::parse(&nested).unwrap().distinct(&roots).is_err());
    let rw=MOUNTS.replace("/source/db ro","/source/db rw");assert!(mounts::Table::parse(&rw).unwrap().require_mount(roots[0],true).is_err());
    for bad in ["bad", "1 0 bad / / rw - ext4 disk rw\n","1 0 8:1 /bad\\999 / rw - ext4 disk rw\n"]{assert!(mounts::Table::parse(bad).is_err());}
}
#[test]
fn escaped_paths_and_longest_mount_prefix_preserve_component_boundaries() {
    let text=format!("{MOUNTS}6 1 8:2 /host\\040data /with\\040space ro shared:1 - ext4 /dev/vdb rw\n");
    let table=mounts::Table::parse(&text).unwrap();
    assert_eq!(table.physical(Path::new("/with space/sub")).unwrap().1,Path::new("/host data/sub"));
    table.distinct(&[Path::new("/source/db"),Path::new("/source/db2")]).unwrap();
}

#[test]
fn deleted_unrelated_mounts_do_not_disable_storage_but_relevant_ones_refuse() {
    let roots=[Path::new("/source/db"),Path::new("/destination/db")];
    let unrelated=format!("{MOUNTS}6 1 8:2 /usr/share/zoneinfo/Asia/Seoul//deleted /etc/localtime ro - ext4 /dev/vdb rw\n");
    mounts::Table::parse(&unrelated).unwrap().distinct(&roots).unwrap();
    let deleted_point=unrelated.replace(" /etc/localtime ro"," /etc/localtime//deleted ro");mounts::Table::parse(&deleted_point).unwrap().distinct(&roots).unwrap();
    for altered in [MOUNTS.replace("/vol/old /source/db","/vol/old//deleted /source/db"),MOUNTS.replace("0:9 / / rw","0:9 /parent//deleted / rw"),format!("{MOUNTS}6 2 8:2 /wal//deleted /source/db/pg_wal ro - ext4 /dev/vdb rw\n")] {
        assert!(mounts::Table::parse(&altered).and_then(|table|table.distinct(&roots)).is_err());
    }
}
#[test]
fn unparseable_nsfs_roots_are_ignored_only_at_unrelated_mountpoints() {
    let roots=[Path::new("/source/db"),Path::new("/destination/db")];
    let unrelated=format!("{MOUNTS}6 1 0:4 net:[4026531992] /run/netns/review rw - nsfs nsfs rw\n");
    mounts::Table::parse(&unrelated).unwrap().distinct(&roots).unwrap();
    for point in ["/","/source","/source/db","/source/db/nested"] {
        let related=unrelated.replace("/run/netns/review",point);
        assert!(mounts::Table::parse(&related).and_then(|table|table.distinct(&roots)).is_err(),"accepted malformed related entry at {point}");
    }
}
#[test]
fn non_utf8_mount_records_preserve_scope_without_lossy_decoding() {
    let roots=[Path::new("/source/db"),Path::new("/destination/db")];
    for entry in [b"6 1 8:2 /host/\xff /unrelated rw - ext4 disk rw\n".as_slice(),b"6 1 8:2 /host /unrelated/\xff rw - ext4 disk rw\n",b"6 1 8:2 /host /unrelated rw - ext4 disk\xff rw\n"] {
        let mut text=MOUNTS.as_bytes().to_vec();text.extend_from_slice(entry);
        mounts::Table::parse_bytes(&text).unwrap().distinct(&roots).unwrap();
    }
    for point in ["/","/source","/source/db","/source/db/nested"] {
        let mut text=MOUNTS.as_bytes().to_vec();text.extend_from_slice(b"6 1 8:2 /host/\xff ");text.extend_from_slice(format!("{point} rw - ext4 disk rw\n").as_bytes());
        assert!(mounts::Table::parse_bytes(&text).and_then(|table|table.distinct(&roots)).is_err());
    }
    let mut nested=MOUNTS.as_bytes().to_vec();nested.extend_from_slice(b"6 1 8:2 /host /source/db/\xff rw - ext4 disk rw\n");
    assert!(mounts::Table::parse_bytes(&nested).unwrap().distinct(&roots).is_err());
    for ambiguous in [b"\xff".as_slice(),b"1 0 8:1 /host relative rw - ext4 disk rw\n"] {assert!(mounts::Table::parse_bytes(ambiguous).is_err());}
}
