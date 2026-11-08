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

/// Repeated semantic edits must keep ordinary editor ownership and labor.
#[test]
fn bulk_excludes_boolean_operator_and_keyword_changes() {
    for (old, new, expected_labor) in [
        ("return true;", "return false;", 2_222_220),
        ("left == right;", "left != right;", 500_000),
        ("if condition", "while condition", 2_500_000),
        ("foo condition", "return condition", 2_500_000),
        ("return condition", "foo condition", 2_500_000),
    ] {
        let before = change(
            &TokenState::default(),
            &format!(
                "{old}
"
            )
            .repeat(5),
            0,
        );
        let edited = change(
            before.state(),
            &format!(
                "{new}
"
            )
            .repeat(5),
            1,
        );
        let ownership = edited.state().ownership().unwrap();
        let labor = edited.labor().unwrap();
        let mut files = vec![edited];
        assert!(!discount_bulk_edits(&mut files, true), "{old} -> {new}");
        assert_eq!(files[0].state().ownership().unwrap(), ownership);
        assert_eq!(files[0].labor().unwrap(), labor);
        assert_eq!(amount(&labor, 1), expected_labor);
        assert!(amount(&ownership, 1) > 0);
    }
}

/// All literal spellings, comment words and indentation are ineligible, not just
/// the three motivating semantic edits. Both sides must qualify as identifiers.
#[test]
fn bulk_excludes_literals_comments_and_indentation() {
    for (old, new) in [
        ("12", "13"),
        ("١٢", "١٣"),
        ("\"old\"", "\"new\""),
        ("'old'", "'new'"),
        ("`old`", "`new`"),
        ("True", "False"),
        ("null", "nil"),
        ("None", "undefined"),
        ("// foo", "// bar"),
        ("# foo", "# bar"),
        ("## foo", "## bar"),
        ("-- foo", "-- bar"),
        ("/* foo */", "/* bar */"),
        (
            "/*
foo
*/",
            "/*
bar
*/",
        ),
    ] {
        let before = change(
            &TokenState::default(),
            &format!(
                "{old}
"
            )
            .repeat(5),
            0,
        );
        let edited = change(
            before.state(),
            &format!(
                "{new}
"
            )
            .repeat(5),
            1,
        );
        let expected = edited.labor().unwrap();
        let mut files = vec![edited];
        assert!(!discount_bulk_edits(&mut files, true), "{old} -> {new}");
        assert_eq!(files[0].labor().unwrap(), expected);
        assert!(amount(&expected, 1) > 0);
    }
    let before = apply_token_change(
        &TokenState::default(),
        &"    foo
"
        .repeat(5),
        &[(AuthorId(0), SCALE)],
        NormalizationMode::IndentationSignificant,
    )
    .unwrap();
    let edited = apply_token_change(
        before.state(),
        &"        foo
"
        .repeat(5),
        &[(AuthorId(1), SCALE)],
        NormalizationMode::IndentationSignificant,
    )
    .unwrap();
    let mut files = vec![edited];
    assert!(!discount_bulk_edits(&mut files, true));
    assert_eq!(amount(&files[0].labor().unwrap(), 1), 2_500_000);
}

/// Candidate filtering must not narrow the denominator or discount matching
/// spellings in comments when a mapping qualifies through code occurrences.
#[test]
fn bulk_keeps_ineligible_weights_and_occurrences_ordinary() {
    let old = "foo
"
    .repeat(5)
        + &"return true;
"
        .repeat(5);
    let new = "bar
"
    .repeat(5)
        + &"return false;
"
        .repeat(5);
    let before = change(&TokenState::default(), &old, 0);
    let mut files = vec![change(before.state(), &new, 1)];
    assert!(!discount_bulk_edits(&mut files, true));
    assert_eq!(amount(&files[0].labor().unwrap(), 1), 7_222_220);

    let old = "foo
"
    .repeat(5)
        + &"// foo
"
        .repeat(3);
    let new = "bar
"
    .repeat(5)
        + &"// bar
"
        .repeat(3);
    let before = change(&TokenState::default(), &old, 0);
    let mut files = vec![change(before.state(), &new, 1)];
    assert!(discount_bulk_edits(&mut files, true));
    assert_eq!(amount(&files[0].labor().unwrap(), 1), 3_400_000);
    assert_eq!(amount(&files[0].state().ownership().unwrap(), 1), 2_400_000);
    assert!(discount_bulk_edits(&mut files, true));
    assert_eq!(amount(&files[0].labor().unwrap(), 1), 3_400_000);
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

/// Exact adoption keeps first-parent removals available but refunds all insertions.
#[test]
fn adopted_token_removals_fund_only_one_copy() {
    let original = change(
        &TokenState::default(),
        "one(value)\ntwo(value)\nthree(value)\nkeep\n",
        0,
    );
    let removed = change(original.state(), "keep\n", 1);
    let mut adopted = change(original.state(), "keep\n", 2);
    adopt_token_parent(&mut adopted, removed.state());
    let mut changes = vec![
        ("a.rs".into(), adopted),
        (
            "b.rs".into(),
            change(
                &TokenState::default(),
                "one( value )\ntwo(value)\nthree(value)\n",
                2,
            ),
        ),
        (
            "c.rs".into(),
            change(
                &TokenState::default(),
                "one(value)\ntwo(value)\nthree(value)\n",
                2,
            ),
        ),
    ];
    let parents = vec![vec![("a.rs".into(), removed)]];
    assert_eq!(reconcile_token_moves(&mut changes, &parents), 12);
    assert_eq!(amount(&changes[1].1.labor().unwrap(), 2), 0);
    assert_eq!(amount(&changes[2].1.labor().unwrap(), 2), 3_000_000);
    assert_eq!(reconcile_token_moves(&mut changes, &parents), 0);
}

/// Parent-only sources obey the same minimum and single-use rules as ordinary moves.
#[test]
fn other_token_parent_supplies_move_sources_once() {
    let original = change(&TokenState::default(), "one\n\ntwo\nthree\n", 0);
    let removed = change(original.state(), "", 1);
    let mut changes = vec![
        (
            "b.rs".into(),
            change(&TokenState::default(), "one\ntwo\nthree\n", 2),
        ),
        (
            "c.rs".into(),
            change(&TokenState::default(), "one\ntwo\nthree\n", 2),
        ),
    ];
    let parents = vec![vec![("a.rs".into(), removed)]];
    assert_eq!(reconcile_token_moves(&mut changes, &parents), 3);
    assert_eq!(
        amount(&changes[0].1.state().ownership().unwrap(), 0),
        3_000_000
    );
    assert_eq!(reconcile_token_moves(&mut changes, &parents), 0);
    assert_eq!(amount(&changes[1].1.labor().unwrap(), 2), 3_000_000);
    let short = change(&TokenState::default(), "one\ntwo\n", 0);
    let mut short_moves = vec![
        ("a.rs".into(), change(short.state(), "", 1)),
        (
            "b.rs".into(),
            change(&TokenState::default(), "one\ntwo\n", 1),
        ),
    ];
    assert_eq!(reconcile_token_moves(&mut short_moves, &[]), 0);
}

/// Merge refunds split exact remaining token fractions among coauthors once.
#[test]
fn token_parent_precedence_and_coauthor_refunds_are_exact() {
    let first = change(&TokenState::default(), "inherited", 0);
    let second = change(&TokenState::default(), "inherited", 1);
    let mut merge = apply_token_change(
        &TokenState::default(),
        "inherited resolution inherited",
        &[(AuthorId(2), 333_333), (AuthorId(3), 666_667)],
        NormalizationMode::IgnoreIndentation,
    )
    .unwrap();
    assert_eq!(
        reconcile_token_parents(&mut merge, [first.state(), second.state()]),
        2
    );
    assert_eq!(reconcile_token_parents(&mut merge, [second.state()]), 0);
    assert_eq!(amount(&merge.labor().unwrap(), 2), 111_110);
    assert_eq!(amount(&merge.labor().unwrap(), 3), 222_223);
    assert_eq!(amount(&merge.state().ownership().unwrap(), 0), 666_666);
    assert_eq!(amount(&merge.state().ownership().unwrap(), 1), 0);
    // Existing first-parent tokens never transfer to the other parent's owner.
    let mut retained = change(first.state(), "inherited resolution", 2);
    assert_eq!(reconcile_token_parents(&mut retained, [second.state()]), 0);
}

/// A disjoint-vocabulary fast path must match the ordinary Myers replacement.
#[test]
fn disjoint_token_rewrite_is_all_editor_work() {
    let original = change(&TokenState::default(), "old unique words", 0);
    let rewritten = change(original.state(), "new distinct text", 1);
    assert_eq!(
        amount(&rewritten.state().ownership().unwrap(), 1),
        1_000_000
    );
    assert_eq!(amount(&rewritten.labor().unwrap(), 1), 1_000_000);
}

/// Partial changed lines cannot inflate a two-line removal into a three-line move.
#[test]
fn token_moves_require_complete_meaningful_source_lines() {
    let original = change(&TokenState::default(), "prefix a\nb\nc\nsuffix\n", 0);
    let mut changes = vec![
        (
            "a.rs".into(),
            change(original.state(), "prefix\nsuffix\n", 1),
        ),
        (
            "b.rs".into(),
            change(&TokenState::default(), "a\nb\nc\n", 1),
        ),
    ];
    assert_eq!(reconcile_token_moves(&mut changes, &[]), 0);
    assert_eq!(amount(&changes[1].1.labor().unwrap(), 1), 3_000_000);
}

#[test]
fn fuzzy_moves_preserve_only_matching_tokens_and_are_idempotent() {
    let text = "fn calculate(value) {\n let result = value + 1;\n return result;\n}\n";
    let original = change(&TokenState::default(), text, 0);
    let renamed = text.replace("value", "input");
    let mut changes = vec![
        ("a.rs".into(), change(original.state(), "", 1)),
        ("b.rs".into(), change(&TokenState::default(), &renamed, 1)),
    ];
    assert!(reconcile_token_moves(&mut changes, &[]) > 0);
    let moved = &changes[1].1;
    for token in moved.state().tokens() {
        let expected = u32::from(token.key().1 == "input");
        assert_eq!(token.owner().authors(), &[(AuthorId(expected), SCALE)]);
    }
    let labor = moved.labor().unwrap();
    assert!(amount(&labor, 1) > 0 && amount(&labor, 1) < 1_000_000);
    assert_eq!(reconcile_token_moves(&mut changes, &[]), 0);
    assert_eq!(changes[1].1.labor().unwrap(), labor);
}

#[test]
fn fuzzy_moves_reject_low_similarity_and_choose_lexical_source_ties() {
    let text = "alpha beta gamma\ndelta epsilon zeta\neta theta iota\n";
    let original = change(&TokenState::default(), text, 0);
    let other = change(&TokenState::default(), text, 2);
    let new = text.replace("gamma", "changed");
    let make = || {
        vec![
            ("z.rs".into(), change(other.state(), "", 1)),
            ("dest.rs".into(), change(&TokenState::default(), &new, 1)),
            ("a.rs".into(), change(original.state(), "", 1)),
        ]
    };
    let mut first = make();
    reconcile_token_moves(&mut first, &[]);
    assert_eq!(amount(&first[1].1.state().ownership().unwrap(), 2), 0);
    assert!(amount(&first[1].1.state().ownership().unwrap(), 0) > 0);
    let expected = first[1].1.state().clone();
    let mut second = make();
    second.reverse();
    reconcile_token_moves(&mut second, &[]);
    assert_eq!(second[1].1.state(), &expected);
    let mut low = vec![
        ("a.rs".into(), change(original.state(), "", 1)),
        (
            "b.rs".into(),
            change(
                &TokenState::default(),
                "alpha new names\ndifferent other words\nno common meaning\n",
                1,
            ),
        ),
    ];
    assert_eq!(reconcile_token_moves(&mut low, &[]), 0);
    assert_eq!(amount(&low[1].1.labor().unwrap(), 1), 3_000_000);
}

#[test]
fn restored_token_provenance_validates_count_without_partial_update() {
    let mut state = change(&TokenState::default(), "one two", 0).into_state();
    let before = state.clone();
    assert_eq!(state.restore_owners(Vec::new()), Err(Error::InvalidShares));
    assert_eq!(state, before);
    state
        .restore_owners(vec![LineOwner::equal(&[AuthorId(1)]).unwrap(); 2])
        .unwrap();
    assert_eq!(amount(&state.ownership().unwrap(), 1), 1_000_000);
}

#[test]
fn fuzzy_same_file_moves_keep_provenance_at_a_different_position() {
    let moved = "fn calculate(value) {\n let result = value + 1;\n return result;\n}\n";
    let anchor = "anchor one two three four five six seven eight nine ten\nsecond stable block with many unique words stays in place\nthird anchor has enough tokens to choose the stationary region\n";
    let before = change(&TokenState::default(), &format!("{moved}{anchor}"), 0);
    let after = format!("{anchor}{}", moved.replace("value", "input"));
    let mut changes = vec![("same.rs".into(), change(before.state(), &after, 1))];
    assert!(reconcile_token_moves(&mut changes, &[]) > 0);
    for token in changes[0].1.state().tokens() {
        let expected = u32::from(token.key().1 == "input");
        assert_eq!(token.owner().authors(), &[(AuthorId(expected), SCALE)]);
    }
}

#[test]
fn fuzzy_threshold_is_inclusive_and_unmatched_lines_cannot_be_paired_again() {
    let text = (0..15).fold(String::new(), |mut text, index| {
        writeln!(text, "word{index}").unwrap();
        text
    });
    let inserted = "new_a\nnew_b\nnew_c\n";
    let new = text.lines().take(12).collect::<Vec<_>>().join("\n") + "\n" + inserted;
    let original = change(&TokenState::default(), &text, 0);
    let competing = change(&TokenState::default(), inserted, 2);
    let mut changes = vec![
        ("a.rs".into(), change(original.state(), "", 1)),
        ("b.rs".into(), change(competing.state(), "", 1)),
        ("c.rs".into(), change(&TokenState::default(), &new, 1)),
    ];
    assert_eq!(reconcile_token_moves(&mut changes, &[]), 12);
    assert_eq!(amount(&changes[2].1.labor().unwrap(), 1), 3_000_000);
    assert_eq!(amount(&changes[2].1.state().ownership().unwrap(), 2), 0);
}
