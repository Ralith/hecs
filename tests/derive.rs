//!

#[cfg(feature = "macros")]
#[cfg_attr(miri, ignore)]
#[test]
fn derive() {
    const TEST_DIR: &str = "tests/derive";
    let t = trybuild::TestCases::new();
    let failures = &["enum.rs", "union.rs"];
    let successes = &[
        "unit_structs.rs",
        "tuple_structs.rs",
        "named_structs.rs",
        "no_prelude.rs",
    ];
    for &passing_test in successes {
        t.pass(&format!("{}/{}", TEST_DIR, passing_test));
    }
    for &failing_test in failures {
        t.compile_fail(&format!("{}/{}", TEST_DIR, failing_test));
    }
}
