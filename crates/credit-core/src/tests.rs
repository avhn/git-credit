//! Exact accounting and boundary tests for the line attribution engine.

use super::*;

const A: AuthorId = AuthorId(0);
const B: AuthorId = AuthorId(1);
const C: AuthorId = AuthorId(2);

/// Apply a conventional whitespace-insensitive edit for one author.
fn edit(prev: &FileState, text: &str, author: AuthorId) -> Change {
    apply_change(
        prev,
        text,
        &[(author, SCALE)],
        NormalizationMode::IgnoreIndentation,
        Policy::default(),
    )
    .unwrap()
}

#[test]
fn formatting_keeps_owners_and_earns_nothing() {
    let root = edit(&FileState::default(), "hello\nworld\n", A);
    let formatted = edit(&root.state, "  hello \r\n\tworld\t\r\n", B);
    assert_eq!(formatted.kind, ChangeKind::Formatting);
    assert!(formatted.labor.is_empty());
    assert_eq!(formatted.state.lines[0].owner.authors(), &[(A, SCALE)]);
    assert_eq!(formatted.state.lines[1].owner.authors(), &[(A, SCALE)]);
}

#[test]
fn minor_edit_retains_owner_with_exact_fractional_labor() {
    let root = edit(&FileState::default(), "abcdefghij", A);
    let changed = edit(&root.state, "abcdefghiX", B);
    assert_eq!(changed.labor[&B], 100_000);
    assert_eq!(changed.state.lines[0].owner.authors(), &[(A, SCALE)]);
    assert_eq!(changed.kind, ChangeKind::Meaningful);
}

#[test]
fn rewrite_transfers_ownership_and_deletion_preserves_labor() {
    let root = edit(&FileState::default(), "aaaa", A);
    let changed = edit(&root.state, "zzzz", B);
    assert_eq!(changed.labor[&B], 1_000_000);
    assert_eq!(changed.state.lines[0].owner.authors(), &[(B, SCALE)]);
    let deleted = edit(&changed.state, "", C);
    assert_eq!(deleted.state.lines, Vec::<Line>::new());
    assert!(deleted.labor.is_empty());
    let mut ledger = Ledger::default();
    ledger.add_labor(&root.labor).unwrap();
    ledger.add_labor(&changed.labor).unwrap();
    ledger.add_labor(&deleted.labor).unwrap();
    ledger.recount([&deleted.state]).unwrap();
    assert!(ledger.ownership.is_empty());
    assert_eq!(ledger.labor, [(A, 1_000_000), (B, 1_000_000)].into());
}

#[test]
fn threshold_is_inclusive_and_validated() {
    let root = edit(&FileState::default(), "abcdefghij", A);
    let changed = edit(&root.state, "abcdefgXYZ", B);
    assert_eq!(changed.state.lines[0].owner.authors(), &[(A, SCALE)]);
    assert_eq!(changed.labor[&B], 300_000);
    let changed = apply_change(
        &root.state,
        "abcdefgXYZ",
        &[(B, SCALE)],
        NormalizationMode::IgnoreIndentation,
        Policy {
            keep_per_mille: 701,
        },
    )
    .unwrap();
    assert_eq!(changed.state.lines[0].owner.authors(), &[(B, SCALE)]);
    assert!(matches!(
        apply_change(
            &root.state,
            "",
            &[(B, SCALE)],
            NormalizationMode::IgnoreIndentation,
            Policy {
                keep_per_mille: 1001
            }
        ),
        Err(Error::InvalidThreshold { .. })
    ));
    assert!(Policy { keep_per_mille: 0 }.validate().is_ok());
    assert!(
        Policy {
            keep_per_mille: 1000
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn indentation_and_interior_spaces_remain_meaningful_when_required() {
    let root = edit(&FileState::default(), "a b", A);
    let significant = apply_change(
        &root.state,
        "    a b",
        &[(B, SCALE)],
        NormalizationMode::IndentationSignificant,
        Policy::default(),
    )
    .unwrap();
    assert_eq!(significant.kind, ChangeKind::Meaningful);
    assert_eq!(significant.labor[&B], 1_000_000);
    assert_eq!(edit(&root.state, "a  b", B).kind, ChangeKind::Meaningful);
    assert_eq!(
        NormalizationMode::IndentationSignificant.normalize("  x\r\t"),
        "  x"
    );
}

#[test]
fn equal_shares_conserve_ownership_and_labor() {
    let owner = LineOwner::equal(&[A, B, C]).unwrap();
    assert_eq!(owner.authors(), &[(A, 333_334), (B, 333_333), (C, 333_333)]);
    let root = apply_change(
        &FileState::default(),
        "abcdefghij",
        owner.authors(),
        NormalizationMode::IgnoreIndentation,
        Policy::default(),
    )
    .unwrap();
    let changed = apply_change(
        &root.state,
        "abcdefghiX",
        owner.authors(),
        NormalizationMode::IgnoreIndentation,
        Policy::default(),
    )
    .unwrap();
    assert_eq!(changed.labor.values().sum::<u64>(), 100_000);
    let mut ledger = Ledger::default();
    ledger.recount([&changed.state]).unwrap();
    assert_eq!(ledger.ownership.values().sum::<u64>(), 1_000_000);
    assert_eq!(ledger.ownership[&A], 333_334);
}

#[test]
fn invalid_shares_are_rejected_at_public_boundary() {
    for shares in [
        vec![],
        vec![(A, 0)],
        vec![(A, SCALE + 1)],
        vec![(A, 500_000), (A, 500_000)],
    ] {
        assert_eq!(LineOwner::new(&shares), Err(Error::InvalidShares));
        assert!(matches!(
            apply_change(
                &FileState::default(),
                "x",
                &shares,
                NormalizationMode::IgnoreIndentation,
                Policy::default()
            ),
            Err(Error::InvalidShares)
        ));
    }
    assert_eq!(LineOwner::equal(&[]), Err(Error::InvalidShares));
    assert_eq!(LineOwner::equal(&[A, A]), Err(Error::InvalidShares));
    assert_eq!(
        LineOwner::equal(&vec![A; 1_000_001]),
        Err(Error::InvalidShares)
    );
    assert!(Error::InvalidShares.to_string().contains("1000000"));
    assert!(Error::Overflow.to_string().contains("overflow"));
}

#[test]
fn ties_use_earliest_position_and_old_lines_are_not_reused() {
    let mut root = edit(&FileState::default(), "abc\nabd", A).state;
    root.lines[1].owner = LineOwner::equal(&[B]).unwrap();
    let changed = apply_change(
        &root,
        "abe\nabf\nabg",
        &[(C, SCALE)],
        NormalizationMode::IgnoreIndentation,
        Policy {
            keep_per_mille: 600,
        },
    )
    .unwrap();
    assert_eq!(changed.state.lines[0].owner.authors(), &[(A, SCALE)]);
    assert_eq!(changed.state.lines[1].owner.authors(), &[(B, SCALE)]);
    assert_eq!(changed.state.lines[2].owner.authors(), &[(C, SCALE)]);
    assert_eq!(changed.labor[&C], 1_666_668);
}

#[test]
fn best_similarity_wins_over_position() {
    let mut root = edit(&FileState::default(), "abcdefghij\n1234567890", A).state;
    root.lines[1].owner = LineOwner::equal(&[B]).unwrap();
    let changed = edit(&root, "123456789X\nabcdefghiX", C);
    assert_eq!(changed.state.lines[0].owner.authors(), &[(B, SCALE)]);
    assert_eq!(changed.state.lines[1].owner.authors(), &[(A, SCALE)]);
    assert_eq!(changed.labor[&C], 200_000);
}

#[test]
fn pairing_cap_is_visible_and_rewrites_hunk() {
    let root = edit(&FileState::default(), &"aaaa\n".repeat(501), A);
    let changed = edit(&root.state, &"aaab\n".repeat(500), B);
    assert!(changed.pairing_capped);
    assert_eq!(changed.labor[&B], 500_000_000);
    assert!(
        changed
            .state
            .lines
            .iter()
            .all(|line| line.owner.authors() == [(B, SCALE)])
    );
}

#[test]
fn blank_empty_and_unicode_lines_have_defined_semantics() {
    assert_eq!(
        edit(&FileState::default(), "", A).state.lines,
        Vec::<Line>::new()
    );
    let blank = edit(&FileState::default(), "\n", A);
    assert_eq!(blank.state.lines.len(), 1);
    assert_eq!(blank.labor[&A], 1_000_000);
    assert_eq!(edit(&blank.state, " \r\n", B).kind, ChangeKind::Formatting);
    let unicode = edit(&FileState::default(), "αβγδε", A);
    assert_eq!(edit(&unicode.state, "αβγδX", B).labor[&B], 200_000);
    let terminal = edit(&FileState::default(), "abc\n", A);
    assert_eq!(edit(&terminal.state, "abc", B).kind, ChangeKind::Formatting);
}

#[test]
fn moves_restore_credit_without_multiplying_it_for_copies() {
    let root = edit(&FileState::default(), "one\ntwo\nthree", A);
    let mut changes = vec![
        ("source".into(), edit(&root.state, "", B)),
        (
            "second".into(),
            edit(&FileState::default(), "one\ntwo\nthree", B),
        ),
        (
            "first".into(),
            edit(&FileState::default(), "one\ntwo\nthree", B),
        ),
    ];
    assert_eq!(reconcile_moves(&mut changes), 3);
    assert_eq!(changes[2].1.labor[&B], 0);
    assert_eq!(changes[1].1.labor[&B], 3_000_000);
    assert_eq!(changes[2].1.state.lines[0].owner.authors(), &[(A, SCALE)]);
    assert_eq!(reconcile_moves(&mut changes), 0);
}

#[test]
fn moves_require_three_consecutive_cross_file_removed_lines() {
    let root = edit(&FileState::default(), "one\ntwo\nthree\nfour\nfive\nsix", A);
    let mut changes = vec![
        ("a".into(), edit(&root.state, "", B)),
        (
            "b".into(),
            edit(&FileState::default(), "one\ntwo\nelse\nfour\nfive\nsix", B),
        ),
    ];
    assert_eq!(reconcile_moves(&mut changes), 3);
    assert_eq!(changes[1].1.labor[&B], 3_000_000);
    let mut copy = vec![(
        "b".into(),
        edit(&FileState::default(), "one\ntwo\nthree", B),
    )];
    assert_eq!(reconcile_moves(&mut copy), 0);
    let mut same = vec![(
        "a".into(),
        edit(&root.state, "four\nfive\nsix\none\ntwo\nthree", B),
    )];
    assert_eq!(reconcile_moves(&mut same), 0);
}

#[test]
fn moves_work_from_changed_files_and_preserve_coauthors() {
    let authors = LineOwner::equal(&[A, C]).unwrap();
    let root = apply_change(
        &FileState::default(),
        "one\ntwo\nthree\nstay",
        authors.authors(),
        NormalizationMode::IgnoreIndentation,
        Policy::default(),
    )
    .unwrap();
    let mut changes = vec![
        ("a".into(), edit(&root.state, "stay", B)),
        (
            "b".into(),
            edit(&FileState::default(), "  one\n two\nthree  ", B),
        ),
    ];
    assert_eq!(reconcile_moves(&mut changes), 3);
    assert_eq!(changes[1].1.state.lines[0].owner, authors);
    assert_eq!(changes[1].1.labor[&B], 0);
}

#[test]
fn overflow_does_not_partially_update_ledger() {
    let mut ledger = Ledger {
        labor: [(A, 4), (B, u64::MAX)].into(),
        ..Ledger::default()
    };
    let original = ledger.clone();
    assert_eq!(
        ledger.add_labor(&[(A, 1), (B, 1)].into()),
        Err(Error::Overflow)
    );
    assert_eq!(ledger, original);
}
