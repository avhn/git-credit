//! Exact token accounting, replacement detection and conservation regressions.

use std::fmt::Write;

use super::*;

/// Build one-editor changes in a non-indentation-sensitive language.
fn change(previous: &TokenState, text: &str, author: u32) -> TokenChange {
    apply_token_change(
        previous,
        text,
        &[(AuthorId(author), SCALE)],
        NormalizationMode::IgnoreIndentation,
    )
    .unwrap()
}

/// Read an author's zero-default fixed-point share without requiring sparse zeros.
fn amount(delta: &LaborDelta, author: u32) -> u64 {
    delta.get(&AuthorId(author)).copied().unwrap_or(0)
}

#[test]
fn rewrap_join_and_trailing_comma_preserve_provenance_without_labor() {
    let original = change(&TokenState::default(), "call(alpha, beta)", 0);
    let wrapped = change(original.state(), "call(\n alpha,\n beta,\n)", 1);
    assert_eq!(wrapped.kind(), ChangeKind::Formatting);
    assert!(wrapped.labor().unwrap().is_empty());
    assert_eq!(amount(&wrapped.state().ownership().unwrap(), 0), 4_000_000);
    assert_eq!(amount(&wrapped.state().ownership().unwrap(), 1), 0);
    let joined = change(wrapped.state(), "call(alpha, beta)", 2);
    assert_eq!(joined.kind(), ChangeKind::Formatting);
    assert!(joined.labor().unwrap().is_empty());
    assert_eq!(joined.state(), original.state());
    let comma = change(original.state(), "call(alpha, beta,)", 1);
    assert_eq!(comma.state(), original.state());
    assert!(comma.labor().unwrap().is_empty());
}

#[test]
fn a_single_replacement_gets_only_its_weighted_share() {
    let original = change(&TokenState::default(), "a b c d e f g h i j", 0);
    let edited = change(original.state(), "a b c d renamed f g h i j", 1);
    assert_eq!(amount(&edited.state().ownership().unwrap(), 0), 900_000);
    assert_eq!(amount(&edited.state().ownership().unwrap(), 1), 100_000);
    assert_eq!(amount(&edited.labor().unwrap(), 1), 100_000);
    let original = change(&TokenState::default(), "value = first;", 0);
    let edited = change(original.state(), "value = second;", 1);
    assert_eq!(amount(&edited.labor().unwrap(), 1), 400_000);
    assert_eq!(amount(&edited.state().ownership().unwrap(), 0), 600_000);
    assert_eq!(amount(&edited.state().ownership().unwrap(), 1), 400_000);
}

#[test]
fn indentation_and_string_contents_are_editor_work() {
    let before = apply_token_change(
        &TokenState::default(),
        "    value",
        &[(AuthorId(0), SCALE)],
        NormalizationMode::IndentationSignificant,
    )
    .unwrap();
    let after = apply_token_change(
        before.state(),
        "        value",
        &[(AuthorId(1), SCALE)],
        NormalizationMode::IndentationSignificant,
    )
    .unwrap();
    assert_eq!(amount(&after.labor().unwrap(), 1), 500_000);
    assert_eq!(amount(&after.state().ownership().unwrap(), 1), 500_000);
    let before = change(&TokenState::default(), "value \"old string\"", 0);
    let after = change(before.state(), "value \"new string\"", 1);
    assert_eq!(amount(&after.labor().unwrap(), 1), 500_000);
    assert_eq!(
        after.state().tokens()[1].owner().authors(),
        &[(AuthorId(1), SCALE)]
    );
    assert_eq!(after.state().tokens()[1].weight(), 1000);
    assert_eq!(after.state().tokens()[1].line(), 0);
    assert_eq!(after.state().tokens()[1].key(), (None, "\"new string\""));
}

#[test]
fn deletions_empty_files_and_zero_weight_lines_earn_nothing() {
    let before = change(&TokenState::default(), "a b\n,\n)", 0);
    assert_eq!(before.state().meaningful_lines(), 2);
    let after = change(before.state(), "a\n\n)", 1);
    assert_eq!(after.kind(), ChangeKind::Meaningful);
    assert!(after.labor().unwrap().is_empty());
    assert_eq!(amount(&after.state().ownership().unwrap(), 0), 2_000_000);
    let deleted = change(after.state(), "", 1);
    assert!(deleted.state().ownership().unwrap().is_empty());
    assert!(deleted.labor().unwrap().is_empty());
    let empty = change(&TokenState::default(), " \n\t\n", 1);
    assert_eq!(empty.kind(), ChangeKind::Formatting);
    assert_eq!(empty.into_state().tokens(), []);
}

/// Ten anchored replacements in three files, with independent lexical contexts.
fn bulk_fixture() -> Vec<TokenChange> {
    [3, 3, 4]
        .into_iter()
        .map(|count| {
            let mut old = String::new();
            for i in 0..count {
                writeln!(old, "prefix{i} foo suffix{i}").unwrap();
            }
            let before = change(&TokenState::default(), &old, 0);
            change(before.state(), &old.replace("foo", "bar"), 1)
        })
        .collect()
}

#[test]
fn bulk_rename_conserves_owners_and_charges_once_per_file() {
    let mut files = bulk_fixture();
    assert!(discount_bulk_edits(&mut files, true));
    for file in &files {
        assert_eq!(amount(&file.labor().unwrap(), 1), 1_000_000);
        assert_eq!(amount(&file.state().ownership().unwrap(), 1), 0);
        assert_eq!(
            amount(&file.state().ownership().unwrap(), 0),
            u64::try_from(file.state().meaningful_lines()).unwrap() * u64::from(SCALE)
        );
    }
    let before: Vec<_> = files.iter().map(|file| file.labor().unwrap()).collect();
    assert!(discount_bulk_edits(&mut files, true));
    assert_eq!(
        before,
        files
            .iter()
            .map(|file| file.labor().unwrap())
            .collect::<Vec<_>>()
    );
    let mut ordinary = bulk_fixture();
    assert!(!discount_bulk_edits(&mut ordinary, false));
    assert_eq!(amount(&ordinary[2].labor().unwrap(), 1), 1_333_332);
    assert_eq!(
        amount(&ordinary[2].state().ownership().unwrap(), 1),
        1_333_336
    );
}

#[test]
fn bulk_requires_count_and_weight_thresholds_and_counts_unpaired_hunks() {
    let before = change(&TokenState::default(), "foo a foo b foo c foo d", 0);
    let mut changes = vec![change(before.state(), "bar a bar b bar c bar d", 1)];
    assert!(!discount_bulk_edits(&mut changes, true));
    let mut files = bulk_fixture();
    // Ten qualifying pairs cannot dominate a large unequal replacement hunk.
    let before = change(&TokenState::default(), "old", 0);
    files.push(change(
        before.state(),
        "one two three four five six seven eight nine ten eleven",
        1,
    ));
    assert!(!discount_bulk_edits(&mut files, true));
    assert!(!discount_bulk_edits(&mut [], true));
    // Five of eight unit-weight replacements exceed 60%; five of nine do not.
    for (extra, expected) in [(3, true), (4, false)] {
        let mut old = String::new();
        let mut new = String::new();
        for i in 0..5 {
            writeln!(old, "anchor{i} foo end{i}").unwrap();
            writeln!(new, "anchor{i} bar end{i}").unwrap();
        }
        for i in 0..extra {
            writeln!(old, "other{i} old{i} tail{i}").unwrap();
            writeln!(new, "other{i} new{i} tail{i}").unwrap();
        }
        let before = change(&TokenState::default(), &old, 0);
        let mut changes = vec![change(before.state(), &new, 1)];
        assert_eq!(discount_bulk_edits(&mut changes, true), expected);
    }
}

/// Removing a comment marker must not transfer an unchanged numeric spelling.
#[test]
fn canonical_text_equality_survives_lexical_category_changes() {
    let before = change(&TokenState::default(), "// 42", 0);
    let after = change(before.state(), "42", 1);
    assert_eq!(amount(&after.state().ownership().unwrap(), 0), 1_000_000);
    assert_eq!(amount(&after.labor().unwrap(), 1), 0);
}

#[test]
fn author_validation_happens_before_attribution() {
    for authors in [
        vec![],
        vec![(AuthorId(0), 0)],
        vec![(AuthorId(0), SCALE - 1)],
        vec![(AuthorId(0), SCALE / 2), (AuthorId(0), SCALE / 2)],
    ] {
        assert_eq!(
            apply_token_change(
                &TokenState::default(),
                "text",
                &authors,
                NormalizationMode::IgnoreIndentation
            )
            .unwrap_err(),
            Error::InvalidShares
        );
    }
}

#[test]
fn conservation_holds_through_many_deterministic_edits_and_coauthors() {
    let mut state = TokenState::default();
    let authors = [
        (AuthorId(1), 333_333),
        (AuthorId(2), 333_333),
        (AuthorId(3), 333_334),
    ];
    for mode in [
        NormalizationMode::IgnoreIndentation,
        NormalizationMode::IndentationSignificant,
    ] {
        for i in 0..100 {
            let text = format!(
                "{}call(value{},\n \"string {}\",\n last{}\n)\n\n",
                " ".repeat(i % 9),
                i % 3,
                i % 7,
                i % 5
            );
            let changed = apply_token_change(&state, &text, &authors, mode).unwrap();
            let total: u64 = changed.state().ownership().unwrap().values().sum();
            assert_eq!(
                total,
                u64::try_from(changed.state().meaningful_lines()).unwrap() * u64::from(SCALE)
            );
            let labor: u64 = changed.labor().unwrap().values().sum();
            assert!(labor <= total);
            state = changed.into_state();
        }
    }
}
