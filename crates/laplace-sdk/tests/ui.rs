// SPDX-License-Identifier: Apache-2.0

#[test]
fn facade_macro_compile_contracts() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/pass/*.rs");
    t.compile_fail("tests/ui/fail/*.rs");
}
