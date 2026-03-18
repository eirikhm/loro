#![allow(deprecated)]
#![allow(unexpected_cfgs)]
use loro::{
    cursor::Cursor, ContainerID, ContainerTrait, EncodedBlobMode, ExportMode, LoroDoc, LoroList,
    LoroText, UndoManager,
};
use std::sync::{Arc, Mutex};
use tracing::{trace, trace_span};

#[ctor::ctor]
fn init() {
    dev_utils::setup_test_log();
}

#[test]
fn test_event_hint_cross_container_merge_bug() {
    let doc = LoroDoc::new();
    let text_a = doc.get_text("text_a");
    let text_b = doc.get_text("text_b");

    // Insert initial content
    text_a.insert(0, "a").unwrap();
    text_b.insert(0, "b").unwrap();
    doc.commit();

    // Track events
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let _guard = doc.subscribe_root(Arc::new(move |batch| {
        for event in batch.events {
            events_clone
                .lock()
                .unwrap()
                .push(event.target.name().to_string());
        }
    }));

    // Delete from both containers - this should generate 2 events
    text_a.delete(0, 1).unwrap();
    text_b.delete(0, 1).unwrap();
    doc.commit();

    // Bug: Only 1 event is generated instead of 2
    let events = events.lock().unwrap();
    assert_eq!(
        events.len(),
        2,
        "Expected 2 events, got {}: {:?}",
        events.len(),
        *events
    );
}

#[test]
fn test_event_hint_bug_reproduction() {
    // This test specifically reproduces the EventHint merge bug
    // by creating delete operations that will be merged incorrectly
    let doc = LoroDoc::new();
    let text_a = doc.get_text("text_a");
    let text_b = doc.get_text("text_b");

    // Insert content
    text_a.insert(0, "hello").unwrap();
    text_b.insert(0, "world").unwrap();
    doc.commit();

    // Track detailed event information
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let _guard = doc.subscribe_root(Arc::new(move |event_batch| {
        let mut events_lock = events_clone.lock().unwrap();

        for event in event_batch.events.iter() {
            let container_name = event.target.name().as_str().to_string();

            if let Some(text_diff) = event.diff.as_text() {
                // Count total operations in the diff
                let mut total_ops = 0;
                let mut delete_ops = 0;
                let mut retain_ops = 0;

                for delta in text_diff.iter() {
                    total_ops += 1;
                    let delta_str = format!("{delta:?}");
                    if delta_str.contains("Delete") {
                        delete_ops += 1;
                    } else if delta_str.contains("Retain") {
                        retain_ops += 1;
                    }
                }

                events_lock.push((container_name, total_ops, delete_ops, retain_ops));
            }
        }
    }));

    // Perform operations that should trigger the bug
    // Delete from position 0 in text_a (deletes 'h')
    text_a.delete(0, 1).unwrap();
    // Delete from position 0 in text_b (deletes 'w')
    text_b.delete(0, 1).unwrap();
    doc.commit();

    let events_lock = events.lock().unwrap();

    println!("\n=== Bug Reproduction Test ===");
    println!("Events received: {:?}", *events_lock);

    // The bug would cause these events to be merged incorrectly
    // We should have 2 events, one for each container
    assert_eq!(
        events_lock.len(),
        2,
        "Should have exactly 2 events, got {}",
        events_lock.len()
    );

    // Each event should only contain operations for its own container
    let text_a_events: Vec<_> = events_lock
        .iter()
        .filter(|(name, _, _, _)| name == "text_a")
        .collect();
    let text_b_events: Vec<_> = events_lock
        .iter()
        .filter(|(name, _, _, _)| name == "text_b")
        .collect();

    assert_eq!(text_a_events.len(), 1, "text_a should have exactly 1 event");
    assert_eq!(text_b_events.len(), 1, "text_b should have exactly 1 event");

    // Check the operations count
    if let Some((_, total_ops, delete_ops, _)) = text_a_events.first() {
        assert_eq!(*total_ops, 1, "text_a should have 1 operation");
        assert_eq!(*delete_ops, 1, "text_a should have 1 delete operation");
    }

    if let Some((_, total_ops, delete_ops, retain_ops)) = text_b_events.first() {
        // text_b might have a retain operation if the bug manifests
        println!(
            "text_b operations - total: {total_ops}, deletes: {delete_ops}, retains: {retain_ops}"
        );
        // If the bug exists, text_b might show unexpected operations
    }

    // Verify final state
    assert_eq!(text_a.to_string(), "ello");
    assert_eq!(text_b.to_string(), "orld");
}

#[test]
fn test_event_hint_merge_bug_clear_demonstration() {
    // This test clearly demonstrates the EventHint merge bug
    let doc = LoroDoc::new();
    let text_a = doc.get_text("text_a");
    let text_b = doc.get_text("text_b");

    // Insert content
    text_a.insert(0, "12345").unwrap();
    text_b.insert(0, "abcde").unwrap();
    doc.commit();

    // Track which containers received events
    let event_containers = Arc::new(Mutex::new(Vec::new()));
    let event_containers_clone = event_containers.clone();

    let _guard = doc.subscribe_root(Arc::new(move |event_batch| {
        let mut containers = event_containers_clone.lock().unwrap();

        println!("\n=== Event Batch ===");
        println!("Total events in batch: {}", event_batch.events.len());

        for (idx, event) in event_batch.events.iter().enumerate() {
            let container_name = event.target.name().as_str().to_string();
            println!("Event #{idx}: Container '{container_name}'");

            if let Some(text_diff) = event.diff.as_text() {
                println!("  Diff operations:");
                for (i, delta) in text_diff.iter().enumerate() {
                    println!("    Operation #{i}: {delta:?}");
                }
            }

            containers.push(container_name);
        }
        println!("=== End Batch ===\n");
    }));

    println!("\nPerforming delete operations:");
    println!("- Deleting position 0 from text_a (removes '1')");
    println!("- Deleting position 0 from text_b (removes 'a')");

    // These two operations should generate two separate events
    // But due to the bug, they might be merged into one
    text_a.delete(0, 1).unwrap();
    text_b.delete(0, 1).unwrap();
    doc.commit();

    let containers = event_containers.lock().unwrap();

    // This assertion will fail if the bug is present
    assert_eq!(
        containers.len(),
        2,
        "Expected 2 events (one for each container), but got {}. Events: {:?}",
        containers.len(),
        *containers
    );

    // Check that both containers received their own events
    let text_a_count = containers.iter().filter(|&c| c == "text_a").count();
    let text_b_count = containers.iter().filter(|&c| c == "text_b").count();

    assert_eq!(text_a_count, 1, "text_a should have exactly 1 event");
    assert_eq!(text_b_count, 1, "text_b should have exactly 1 event");

    // Verify the final state is correct
    assert_eq!(text_a.to_string(), "2345");
    assert_eq!(text_b.to_string(), "bcde");
}

#[test]
fn test_undo_counter_after_remote_update_issue_905() {
    let doc_a = LoroDoc::new();
    doc_a.set_peer_id(1).unwrap();
    let mut undo_manager = UndoManager::new(&doc_a);
    undo_manager.set_merge_interval(0);

    let counter_a = doc_a.get_counter("counter");
    counter_a.increment(1.0).unwrap();
    doc_a.commit();

    let doc_b = LoroDoc::new();
    doc_b.set_peer_id(2).unwrap();
    doc_b.import(&doc_a.export(ExportMode::all_updates()).unwrap())
        .unwrap();

    let counter_b = doc_b.get_counter("counter");
    assert_eq!(counter_b.get_value(), 1.0);
    counter_b.increment(1.0).unwrap();
    doc_b.commit();

    doc_a.import(&doc_b.export(ExportMode::all_updates()).unwrap())
        .unwrap();
    assert_eq!(counter_a.get_value(), 2.0);

    assert!(undo_manager.can_undo());
    assert!(undo_manager.undo().unwrap());
    assert_eq!(counter_a.get_value(), 1.0);

    assert!(undo_manager.can_redo());
    assert!(undo_manager.redo().unwrap());
    assert_eq!(counter_a.get_value(), 2.0);
}

#[test]
fn import_twice() {
    let doc = LoroDoc::new();
    let base64 = "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";
    let decoded_bytes = base64::decode(base64).expect("base64 decode error");
    doc.import(&decoded_bytes).unwrap();
    doc.import(&decoded_bytes).unwrap();
}

#[test]
fn import_doc_err() {
    let base64 = include_bytes!("./issue_import.base64.txt");
    let base64 = str::from_utf8(base64).unwrap();
    let decoded_bytes = base64::decode(base64).expect("base64 decode error");

    let doc = LoroDoc::new();
    doc.import(&decoded_bytes).unwrap();
    dbg!(doc.get_deep_value());
}

#[test]
fn undo_tree_mov_between_children() {
    let doc = LoroDoc::new();
    let mut undo = UndoManager::new(&doc);
    let tree = doc.get_tree("tree");
    let a = tree.create(None).unwrap();
    tree.get_meta(a).unwrap().insert("title", "A").unwrap();
    doc.commit();
    let b = tree.create(None).unwrap();
    tree.get_meta(b).unwrap().insert("title", "B").unwrap();
    doc.commit();
    let doc_value_0 = doc.get_deep_value();
    tree.mov_after(a, b).unwrap();
    undo.undo().unwrap();
    let doc_value_1 = doc.get_deep_value();
    assert_eq!(doc_value_0, doc_value_1);
}

#[test]
fn issue_822_tree_shallow_snapshot_roundtrip() {
    let snapshot_bytes = include_bytes!("./issue_822.bin");
    let doc = LoroDoc::new();
    doc.import(snapshot_bytes).expect("import snapshot blob");

    let tree = doc.get_tree("nodes");
    let tree_before = tree.get_value();
    let doc_before = doc.get_value();

    let snapshot_meta =
        LoroDoc::decode_import_blob_meta(snapshot_bytes, false).expect("decode snapshot meta");
    assert!(snapshot_meta.mode.is_snapshot());
    let imported_is_shallow = snapshot_meta.mode == EncodedBlobMode::ShallowSnapshot;

    let frontiers = doc.state_frontiers();
    let shallow_bytes = trace_span!("EXPORT").in_scope(|| {
        doc.export(ExportMode::shallow_snapshot(&frontiers))
            .expect("export shallow snapshot")
    });

    let snapshot_meta_1 = LoroDoc::decode_import_blob_meta(&shallow_bytes, false).unwrap();
    assert!(matches!(
        snapshot_meta_1.mode,
        EncodedBlobMode::ShallowSnapshot
    ));

    let shallow_meta =
        LoroDoc::decode_import_blob_meta(&shallow_bytes, false).expect("decode shallow meta");
    assert_eq!(shallow_meta.mode, EncodedBlobMode::ShallowSnapshot);

    let shallow_doc = LoroDoc::new();
    trace_span!("FINAL_IMPORT").in_scope(|| {
        trace!("bytes.len: {}", shallow_bytes.len());
        shallow_doc
            .import(&shallow_bytes)
            .expect("import shallow snapshot");
    });

    assert!(shallow_doc.is_shallow());
    assert_eq!(doc.is_shallow(), imported_is_shallow);

    let tree_after = shallow_doc.get_tree("nodes").get_value();
    let doc_after = shallow_doc.get_value();

    assert_eq!(
        tree_before, tree_after,
        "tree shallow value should roundtrip"
    );
    assert_eq!(doc_before, doc_after, "doc shallow value should roundtrip");
}

#[test]
fn fix_get_unknown_cursor_position() {
    let doc = LoroDoc::new();
    let pos = doc.get_cursor_pos(&Cursor::new(
        None,
        ContainerID::Normal {
            peer: 10,
            counter: 0,
            container_type: loro::ContainerType::List,
        },
        loro::cursor::Side::Left,
        0,
    ));
    assert!(matches!(pos, Err(..)));
}

#[test]
fn issue_924_fork_shallow_snapshot() {
    let doc_a = LoroDoc::new();
    let list_a = doc_a.get_list("list");
    list_a.insert(0, "A").unwrap();
    list_a.insert(1, "B").unwrap();
    list_a.insert(2, "C").unwrap();

    let bytes = doc_a
        .export(ExportMode::shallow_snapshot(&doc_a.oplog_frontiers()))
        .unwrap();

    let doc_b = LoroDoc::new();
    doc_b.import(&bytes).unwrap();

    assert!(doc_b.is_shallow());
    assert!(!doc_b.is_detached());

    let doc_c = doc_b.fork();
    assert!(doc_c.is_shallow());
    assert_eq!(doc_b.get_deep_value(), doc_c.get_deep_value());
}

/// Regression test for ensure_vv_for double-set panic (loro_dag.rs:916).
///
/// Reproduces the crash:
///   panicked at crates/loro-internal/src/oplog/loro_dag.rs:916:49:
///   called `Result::unwrap()` on an `Err` value: ImVersionVector(...)
///
/// The ensure_vv_for stack-based DFS can push the same Arc<AppDagNodeInner>
/// twice when the DAG has a diamond pattern where a dep node appears in
/// both the direct deps and the transitive deps of another dep.
/// The second OnceCell::set() panics because the VV was already computed.
///
/// Pattern: D deps=[X_batch2, Y_batch2, Z], Z deps=[X_batch1, Y_batch1]
/// where X_batch1 and X_batch2 are from the same peer (may be same node
/// if not split), and similarly for Y.
#[test]
fn issue_ensure_vv_for_double_set_panic() {
    // Three peers simulate calendar sync writers.
    let peer_x: u64 = 5949460327480635965;
    let peer_y: u64 = 453872192370119494;
    let peer_z: u64 = 5949460327480677794;

    // Peer X writes batch 1.
    let doc_x = LoroDoc::new();
    doc_x.set_peer_id(peer_x).unwrap();
    let map_x = doc_x.get_map("events");
    for i in 0..50 {
        map_x
            .insert(&format!("x1-{}", i), format!("X batch1 event {}", i))
            .unwrap();
    }
    doc_x.commit();
    let x_batch1 = doc_x.export(ExportMode::all_updates()).unwrap();
    let x_batch1_vv = doc_x.oplog_vv();

    // Peer Y writes batch 1.
    let doc_y = LoroDoc::new();
    doc_y.set_peer_id(peer_y).unwrap();
    let map_y = doc_y.get_map("events");
    for i in 0..50 {
        map_y
            .insert(&format!("y1-{}", i), format!("Y batch1 event {}", i))
            .unwrap();
    }
    doc_y.commit();
    let y_batch1 = doc_y.export(ExportMode::all_updates()).unwrap();
    let y_batch1_vv = doc_y.oplog_vv();

    // Peer Z imports X_batch1 + Y_batch1, then writes.
    // Z now depends on both X_batch1_last and Y_batch1_last.
    let doc_z = LoroDoc::new();
    doc_z.set_peer_id(peer_z).unwrap();
    doc_z.import(&x_batch1).unwrap();
    doc_z.import(&y_batch1).unwrap();
    let map_z = doc_z.get_map("events");
    for i in 0..10 {
        map_z
            .insert(&format!("z-{}", i), format!("Z event {}", i))
            .unwrap();
    }
    doc_z.commit();
    let z_updates = doc_z.export(ExportMode::all_updates()).unwrap();

    // Peer X writes batch 2 (doesn't know about Z).
    for i in 0..50 {
        map_x
            .insert(&format!("x2-{}", i), format!("X batch2 event {}", i))
            .unwrap();
    }
    doc_x.commit();
    let x_batch2 = doc_x
        .export(ExportMode::updates_owned(x_batch1_vv))
        .unwrap();

    // Peer Y writes batch 2 (doesn't know about Z).
    for i in 0..50 {
        map_y
            .insert(&format!("y2-{}", i), format!("Y batch2 event {}", i))
            .unwrap();
    }
    doc_y.commit();
    let y_batch2 = doc_y
        .export(ExportMode::updates_owned(y_batch1_vv))
        .unwrap();

    // Merge everything into one document.
    // After merge: frontiers = [X_batch2_last, Y_batch2_last, Z_last]
    // because Z only depends on batch1 of both X and Y.
    let merged = LoroDoc::new();
    merged.import(&x_batch1).unwrap();
    merged.import(&x_batch2).unwrap();
    merged.import(&y_batch1).unwrap();
    merged.import(&y_batch2).unwrap();
    merged.import(&z_updates).unwrap();

    let old_frontiers = merged.oplog_frontiers();
    assert!(
        old_frontiers.len() >= 3,
        "should have 3+ frontier entries (X_batch2, Y_batch2, Z), got {}",
        old_frontiers.len()
    );

    // New peer writes on the merged doc, creating a commit that depends
    // on all 3 frontiers. This is the node that triggers ensure_vv_for
    // with the problematic diamond pattern.
    merged.set_peer_id(12345).unwrap();
    merged
        .get_map("events")
        .insert("merged-event", "merged")
        .unwrap();
    merged.commit();
    let new_frontiers = merged.oplog_frontiers();

    // Export snapshot. Reload to force lazy DAG node loading.
    let snapshot = merged.export(ExportMode::Snapshot).unwrap();
    let reloaded = LoroDoc::new();
    reloaded.import(&snapshot).unwrap();

    // Free caches to clear any precomputed VVs.
    reloaded.free_diff_calculator();
    reloaded.free_history_cache();

    // This Diff call triggers ensure_vv_for on the lazily-loaded DAG.
    // With the diamond pattern (Z deps on X_batch1 and Y_batch1, while
    // the merge commit deps on X_batch2, Y_batch2, and Z), the DFS may
    // push the same node twice, causing the OnceCell double-set panic.
    let result = reloaded.diff(&old_frontiers, &new_frontiers);
    assert!(
        result.is_ok(),
        "diff should not panic: {:?}",
        result.err()
    );
}

/// Diff with old frontiers before shallow root should return an error, not panic.
#[test]
fn issue_diff_shallow_snapshot_should_not_panic() {
    let peer_x: u64 = 5949460327480635965;
    let peer_y: u64 = 453872192370119494;
    let peer_z: u64 = 5949460327480677794;

    let doc_x = LoroDoc::new();
    doc_x.set_peer_id(peer_x).unwrap();
    for i in 0..100 {
        doc_x
            .get_map("events")
            .insert(&format!("x1-{}", i), format!("X1 {}", i))
            .unwrap();
    }
    doc_x.commit();
    let x_batch1 = doc_x.export(ExportMode::all_updates()).unwrap();
    let x_b1_vv = doc_x.oplog_vv();

    let doc_y = LoroDoc::new();
    doc_y.set_peer_id(peer_y).unwrap();
    for i in 0..100 {
        doc_y
            .get_map("events")
            .insert(&format!("y1-{}", i), format!("Y1 {}", i))
            .unwrap();
    }
    doc_y.commit();
    let y_batch1 = doc_y.export(ExportMode::all_updates()).unwrap();
    let y_b1_vv = doc_y.oplog_vv();

    // Z imports both batch1s.
    let doc_z = LoroDoc::new();
    doc_z.set_peer_id(peer_z).unwrap();
    doc_z.import(&x_batch1).unwrap();
    doc_z.import(&y_batch1).unwrap();
    for i in 0..20 {
        doc_z
            .get_map("events")
            .insert(&format!("z-{}", i), format!("Z {}", i))
            .unwrap();
    }
    doc_z.commit();
    let z_updates = doc_z.export(ExportMode::all_updates()).unwrap();

    // X and Y write more (batch2).
    for i in 0..100 {
        doc_x
            .get_map("events")
            .insert(&format!("x2-{}", i), format!("X2 {}", i))
            .unwrap();
    }
    doc_x.commit();
    let x_batch2 = doc_x.export(ExportMode::updates_owned(x_b1_vv)).unwrap();

    for i in 0..100 {
        doc_y
            .get_map("events")
            .insert(&format!("y2-{}", i), format!("Y2 {}", i))
            .unwrap();
    }
    doc_y.commit();
    let y_batch2 = doc_y.export(ExportMode::updates_owned(y_b1_vv)).unwrap();

    // Merge all.
    let merged = LoroDoc::new();
    merged.import(&x_batch1).unwrap();
    merged.import(&x_batch2).unwrap();
    merged.import(&y_batch1).unwrap();
    merged.import(&y_batch2).unwrap();
    merged.import(&z_updates).unwrap();

    let old_frontiers = merged.oplog_frontiers();

    // Merge commit.
    merged.set_peer_id(99999).unwrap();
    merged
        .get_map("events")
        .insert("final", "done")
        .unwrap();
    merged.commit();
    let new_frontiers = merged.oplog_frontiers();

    // Export as shallow snapshot at current frontiers.
    let shallow = merged
        .export(ExportMode::shallow_snapshot(&merged.oplog_frontiers()))
        .unwrap();
    let reloaded = LoroDoc::new();
    reloaded.import(&shallow).unwrap();

    // Old frontiers predate the shallow root — Diff should return Err, not panic.
    let result = reloaded.diff(&old_frontiers, &new_frontiers);
    assert!(
        result.is_err(),
        "diff with pre-shallow frontiers should return error, not panic"
    );

    // Diff between post-shallow frontiers should work fine.
    let post_shallow_old = reloaded.oplog_frontiers();
    let result2 = reloaded.diff(&post_shallow_old, &new_frontiers);
    assert!(result2.is_ok(), "diff with valid post-shallow frontiers should work");
}

#[test]
fn get_unknown_cursor_position_but_its_in_pending() {
    let doc_0 = LoroDoc::new();
    let list = doc_0
        .get_map("map")
        .insert_container("list", LoroList::new())
        .unwrap();
    let v = doc_0.oplog_vv();
    let text = list.insert_container(0, LoroText::new()).unwrap();
    text.insert(0, "h").unwrap();
    doc_0.commit();
    text.insert(1, "heihei").unwrap();
    let updates = doc_0.export(ExportMode::updates_owned(v)).unwrap();

    let doc_1 = LoroDoc::new();
    let import_status = doc_1.import(&updates).unwrap();
    assert!(import_status.pending.is_some());
    assert!(doc_1.get_container(text.id()).is_none());
    assert!(!doc_1.has_container(&text.id()));
    assert_eq!(doc_1.get_path_to_container(&text.id()), None);
}
